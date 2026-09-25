//! Pre-validation of fields libpod validates on its own.
//!
//! libpod validates a handful of fields when a `SpecGenerator` arrives over
//! the create endpoint and when a build query string arrives at the build
//! endpoint. The validators are scattered across the libpod source: namespaces
//! via `ParseNamespace`, `device_cgroup_rule` access strings via
//! `parseLinuxResourcesDeviceAccess`, and a handful of others (build-arg keys,
//! label keys) rejected by the buildkit-fronted parser. When libpod rejects
//! one of these, the raw error message names the validator, not the
//! compose-side field. Podup owns the compose-side translation, so it also
//! owns the field-aware error surface: this module runs the same allow-lists
//! libpod uses, produces a structured `PodmanError::Field` before contacting
//! the daemon, and lets every other field fall through to the runtime.
//!
//! `#1357` reframed the original "per-field allow-list for compose fields
//! forwarded to libpod" proposal into this: pre-validate what libpod
//! pre-validates, name the field and value, and never invent allow-lists for
//! the rest.

use std::fmt::Write as _;

use crate::engine::container::{device_spec_parts, parse_device_cgroup_rule};
use crate::error::ComposeError;

use super::error::PodmanError;

/// Compose-side field names for each `SpecGenerator` namespace slot.
const PID_FIELD: &str = "pid";
const IPC_FIELD: &str = "ipc";
const UTS_FIELD: &str = "uts";
const USERNS_FIELD: &str = "userns_mode";
const CGROUP_FIELD: &str = "cgroup";

/// The namespace modes every slot accepts, in the form a compose-side string
/// would be in.
///
/// This list used to be the whole allow-list, shared by all five slots, and
/// that made it wrong for four of them. Measured against podman 5.7.0, only
/// `host`, `private` and `pod` are universal: `none` and `shareable` parse for
/// `ipc` alone, and `keep-id`/`auto`/`nomap` for `userns_mode` alone. The
/// per-slot extras live in [`IPC_EXTRA_MODES`] and [`USERNS_EXTRA_MODES`].
///
/// The measurement distinguishes a mode podman refuses to *parse* (it says
/// "unrecognized namespace mode") from one that parses and then fails to
/// apply. On a rootless host `--userns=auto` reports "not enough unused IDs in
/// user namespace" and `--userns=private` wants a UID mapping; both are valid
/// modes that this host cannot satisfy, and neither belongs in a syntax
/// allow-list.
///
/// `container:<id>` joins another container's namespace (compose's
/// `container:NAME` and `service:NAME` forms; podup rewrites service→container
/// before this list sees it). The `ns:<path>` form joins a namespace by an
/// absolute filesystem path, directly user-facing on the compose side, so it
/// has to be allowed.
///
/// `network_mode` is intentionally **not** validated here: the engine
/// translates `service:NAME` to `container:<cname>` and accepts `bridge`,
/// which is a libpod netns mode but is not a member of the strict pid/ipc/uts/
/// user/cgroup allow-list. Validating `network_mode` against this list would
/// reject a working compose file. The engine validates the *result* of the
/// translation post-hoc; a rejected value still surfaces through the
/// `netns` field of the rendered error, just via the libpod message rather
/// than the pre-validator.
const NS_MODES: &[&str] = &["host", "private", "pod"];

/// `ipc` takes two modes the other slots reject. `shareable` is the one
/// compose files actually reach for: it is what lets a second container join
/// this one's IPC namespace later, and podup used to refuse it outright.
const IPC_EXTRA_MODES: &[&str] = &["none", "shareable"];

/// `userns_mode` has its own vocabulary, and it is the reason this list had to
/// stop being shared. `keep-id` maps the calling user into the container and
/// is the standard rootless answer to a file-ownership mismatch; `nomap` maps
/// nothing; `auto` lets podman pick a free range.
const USERNS_EXTRA_MODES: &[&str] = &["keep-id", "auto", "nomap"];

/// `container:` is not a value in the allow-list; it must carry an id.
/// `ns:` is not a value either; it must carry a path. The presence of the
/// prefix is the test; the suffix is whatever the user typed.
const NS_PREFIX_MODES: &[&str] = &["container:", "ns:"];

/// `keep-id` and `auto` also take options, as `keep-id:uid=1000,gid=1000` or
/// `auto:size=65536`. Only `userns_mode` accepts these.
const USERNS_PREFIX_MODES: &[&str] = &["keep-id:", "auto:"];

/// Per-namespace validator entry: the compose-side field name and the mode
/// value to validate. `None` means the namespace slot was unset (skip).
type NsSlot<'a> = (&'a str, Option<&'a str>);

/// Validate the namespace slots a compose service set, against the same
/// allow-list libpod's `ParseNamespace` accepts. Returns the first failing
/// slot as a `(field, value, allowed_modes)` triple so the caller can format
/// a `Field` error, or `None` when every slot is either unset or accepted.
pub(crate) fn first_invalid_namespace(slots: &[NsSlot<'_>]) -> Option<(String, String, String)> {
	for (field, value) in slots {
		let Some(mode) = value else { continue };
		if is_valid_namespace_mode(field, mode) {
			continue;
		}
		return Some((
			(*field).to_string(),
			(*mode).to_string(),
			allowed_namespace_modes(field),
		));
	}
	None
}

/// The plain modes this slot accepts on top of [`NS_MODES`].
fn extra_modes(field: &str) -> &'static [&'static str] {
	match field {
		IPC_FIELD => IPC_EXTRA_MODES,
		USERNS_FIELD => USERNS_EXTRA_MODES,
		_ => &[],
	}
}

/// The `prefix:suffix` forms this slot accepts on top of [`NS_PREFIX_MODES`].
fn extra_prefixes(field: &str) -> &'static [&'static str] {
	if field == USERNS_FIELD {
		USERNS_PREFIX_MODES
	} else {
		&[]
	}
}

fn is_valid_namespace_mode(field: &str, mode: &str) -> bool {
	if NS_MODES.contains(&mode) || extra_modes(field).contains(&mode) {
		return true;
	}
	if let Some((name, options)) = mode.split_once(':') {
		if field == USERNS_FIELD && matches!(name, "auto" | "keep-id") {
			return !options.is_empty();
		}
	}
	// Measure the suffix against the prefix that actually matched. The old
	// code compared every prefix against `"container:".len()`, which would
	// have rejected a short but legal `ns:/x`.
	for p in NS_PREFIX_MODES {
		if mode.starts_with(p) {
			return mode.len() > p.len();
		}
	}
	false
}

fn allowed_namespace_modes(field: &str) -> String {
	let mut s = String::from("one of ");
	let mut first = true;
	let sep = |s: &mut String, first: &mut bool| {
		if !*first {
			s.push_str(", ");
		}
		*first = false;
	};
	for m in NS_MODES.iter().chain(extra_modes(field)) {
		sep(&mut s, &mut first);
		write!(&mut s, "`{m}`").expect("writing to String never fails");
	}
	for p in NS_PREFIX_MODES.iter().chain(extra_prefixes(field)) {
		sep(&mut s, &mut first);
		write!(&mut s, "`{p}<id-or-path>`").expect("writing to String never fails");
	}
	s
}

// ---------------------------------------------------------------------------
// device_cgroup_rule access validation
// ---------------------------------------------------------------------------

/// Validate a `device_cgroup_rule` access string against libpod's
/// `parseLinuxResourcesDeviceAccess`. The OCI runtime-spec allows any
/// combination of `r`, `w`, `m` (read, write, mknod); a non-empty access
/// string that is not a subset of those three letters is rejected.
///
/// The strings come from the engine's parsers (`parse_device_cgroup_rule`
/// for `device_cgroup_rules:` and `device_spec_parts` for `devices:`), so
/// the set passed here is exactly what the live up path would build into a
/// `LinuxDeviceCgroup`. A malformed rule never reaches this function: the
/// engine skips it with a warning, and so do we.
pub(crate) fn first_invalid_device_access<'a, I>(rules: I) -> Option<(String, String)>
where
	I: IntoIterator<Item = &'a str>,
{
	for (i, access) in rules.into_iter().enumerate() {
		if access.is_empty() {
			continue;
		}
		if !is_valid_device_access(access) {
			return Some((
				format!("device_cgroup_rule[{i}].access"),
				access.to_string(),
			));
		}
	}
	None
}

fn is_valid_device_access(access: &str) -> bool {
	!access.is_empty()
		&& access.chars().all(|c| matches!(c, 'r' | 'w' | 'm'))
		&& access.chars().any(|c| matches!(c, 'r' | 'w' | 'm'))
}

// ---------------------------------------------------------------------------
// build query key validation
// ---------------------------------------------------------------------------

/// The OCI build-arg / build-label key charset libpod accepts.
///
/// libpod's buildkit-fronted parser rejects keys that contain any character
/// outside `[A-Za-z0-9_.-]`. Validating client-side lets a bad key surface as
/// a `build.args: ... (value: "...")` error naming the field, instead of
/// libpod's `400` body (which names the key but not the compose-side field)
/// (#1357).
fn is_valid_kv_key(key: &str) -> bool {
	!key.is_empty()
		&& key
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

/// Validate every key in a `build.args` / `build.labels` map. Returns the
/// first invalid key as `(field, key, message)` so the caller can build a
/// `Field` error.
pub(crate) fn first_invalid_kv_key<'a, I>(field: &str, keys: I) -> Option<(String, String, String)>
where
	I: IntoIterator<Item = &'a str>,
{
	for key in keys {
		if is_valid_kv_key(key) {
			continue;
		}
		return Some((
			field.to_string(),
			key.to_string(),
			format!("key {key:?} is not a valid identifier; must match `[A-Za-z0-9_.-]+`"),
		));
	}
	None
}

// ---------------------------------------------------------------------------
// value rendering
// ---------------------------------------------------------------------------

/// Truncate a value for inclusion in a `Field` error so a huge or binary value
/// does not flood the rendered message. Multi-line values are collapsed onto
/// one line so the message stays single-line. The threshold is generous
/// (256 chars), long enough for any realistic compose value, short enough
/// to keep the error readable.
pub(crate) fn render_value(value: &str) -> String {
	let mut s = String::with_capacity(value.len().min(256));
	for (i, c) in value.chars().enumerate() {
		if i >= 256 {
			s.push('…');
			break;
		}
		match c {
			'\n' => s.push_str("\\n"),
			'\r' => s.push_str("\\r"),
			'\t' => s.push_str("\\t"),
			c if c.is_control() => write!(&mut s, "\\u{{{:x}}}", c as u32).unwrap(),
			c => s.push(c),
		}
	}
	s
}

// ---------------------------------------------------------------------------
// build field error
// ---------------------------------------------------------------------------

/// Construct a `PodmanError::Field` for a build-query parameter.
pub(crate) fn build_field_error(
	field: impl Into<String>,
	value: impl Into<String>,
	message: impl Into<String>,
) -> PodmanError {
	let value_str = value.into();
	PodmanError::Field {
		service: String::new(),
		field: field.into(),
		value: render_value(&value_str),
		message: message.into(),
	}
}

/// Construct a `PodmanError::Field` for a `SpecGenerator` field that podup
/// pre-validated.
pub(crate) fn spec_field_error(
	service: impl Into<String>,
	field: impl Into<String>,
	value: impl Into<String>,
	message: impl Into<String>,
) -> PodmanError {
	let value_str = value.into();
	PodmanError::Field {
		service: service.into(),
		field: field.into(),
		value: render_value(&value_str),
		message: message.into(),
	}
}

/// Pre-validate the `SpecGenerator` fields libpod validates on its own, so a
/// rejected value surfaces as a `PodmanError::Field` carrying the compose
/// field name and offending value instead of libpod's raw validator text.
///
/// Called from `run_up` before any network, volume, secret, pod or container
/// is created (#1867) and again from the per-service `create_and_start`
/// path: the per-service call is a belt to the up-front call's braces,
/// keeping the field-shaped error if a future change introduces a service
/// whose own pre-validation grew a field the up-front loop has not been
/// taught yet. The two callers compute the same allow-list against the same
/// compose-side value, so the answer always agrees.
pub(crate) fn pre_validate_spec(
	service_name: &str,
	service: &crate::compose::types::Service,
) -> Result<(), ComposeError> {
	// 1. Namespace modes for the slots whose compose string is forwarded
	//    verbatim to `ParseNamespace`. `network_mode` is omitted: the engine
	//    translates it (`service:NAME` → `container:<cname>`, plus `bridge`
	//    is a valid netns-only mode not in the strict allow-list), so the
	//    validator would reject a working compose file. Its result is checked
	//    post-translation when the spec is built.
	let slots: Vec<(&str, Option<&str>)> = vec![
		(PID_FIELD, service.pid.as_deref()),
		(IPC_FIELD, service.ipc.as_deref()),
		(UTS_FIELD, service.uts.as_deref()),
		(USERNS_FIELD, service.userns_mode.as_deref()),
		(CGROUP_FIELD, service.cgroup.as_deref()),
	];
	if let Some((field, value, allowed)) = first_invalid_namespace(&slots) {
		let msg = format!("namespace mode {value:?} is not recognised; must be {allowed}");
		return Err(ComposeError::Podman(spec_field_error(
			service_name,
			field,
			value,
			msg,
		)));
	}

	// 2. device_cgroup_rule access strings. The engine's parsers
	//    (`parse_device_cgroup_rule` for `device_cgroup_rules:`, and
	//    `device_spec_parts` for `devices:`) are the source of truth for
	//    what counts as a valid rule: a four-field rule is malformed and
	//    the engine warns and drops it, so the validator must not surface
	//    its third token as an "invalid access". Calling the same parsers
	//    the spec builder calls keeps the two paths in lockstep.
	let mut access: Vec<String> =
		Vec::with_capacity(service.device_cgroup_rules.len() + service.devices.len());
	for raw in &service.device_cgroup_rules {
		if let Some(rule) = parse_device_cgroup_rule(raw) {
			if let Some(a) = rule.access.as_deref() {
				access.push(a.to_string());
			}
		}
	}
	for raw in &service.devices {
		let (_host, _cont, a) = device_spec_parts(raw);
		if let Some(a) = a {
			access.push(a.to_string());
		}
	}
	if let Some((field, value)) = first_invalid_device_access(access.iter().map(String::as_str)) {
		let msg =
			format!("access string {value:?} is not one of `r`, `w`, `m` or a combination thereof");
		return Err(ComposeError::Podman(spec_field_error(
			service_name,
			field,
			value,
			msg,
		)));
	}

	Ok(())
}

/// Pre-validate the build-query fields libpod validates. Called from the
/// build path before the URL is assembled, so a bad key fails before any
/// POST to the daemon (#1357).
///
/// The `labels` map is iterated for key validation only, so it can be
/// any map type (`HashMap`, `BTreeMap`, ...) the caller wants; the
/// build path uses a `BTreeMap` to keep label order deterministic
/// across builds so a second `podup build` of the same Containerfile
/// hits the buildkit layer cache.
pub(crate) fn pre_validate_build(
	build_args: &std::collections::HashMap<String, String>,
	labels: &impl LabelKeys,
) -> Result<(), ComposeError> {
	if let Some((field, key, msg)) =
		first_invalid_kv_key("build.args", build_args.keys().map(String::as_str))
	{
		return Err(ComposeError::Podman(build_field_error(field, key, msg)));
	}
	if let Some((field, key, msg)) =
		first_invalid_kv_key("build.labels", labels.keys().map(String::as_str))
	{
		return Err(ComposeError::Podman(build_field_error(field, key, msg)));
	}
	Ok(())
}

/// Any map whose `keys()` iterator yields `String`s, with the
/// pre-validation helper's call shape (`keys().map(String::as_str)`).
/// Implemented for `HashMap<String, String>` and `BTreeMap<String,
/// String>` so the build path can pick a deterministic ordering
/// without copying into the validation helper's preferred type.
pub(crate) trait LabelKeys {
	type Iter<'a>: Iterator<Item = &'a String>
	where
		Self: 'a;
	fn keys(&self) -> Self::Iter<'_>;
}

impl LabelKeys for std::collections::HashMap<String, String> {
	type Iter<'a>
		= std::collections::hash_map::Keys<'a, String, String>
	where
		Self: 'a;
	fn keys(&self) -> Self::Iter<'_> {
		self.keys()
	}
}

impl LabelKeys for std::collections::BTreeMap<String, String> {
	type Iter<'a>
		= std::collections::btree_map::Keys<'a, String, String>
	where
		Self: 'a;
	fn keys(&self) -> Self::Iter<'_> {
		self.keys()
	}
}

#[cfg(test)]
#[path = "validate_tests.rs"]
mod tests;
