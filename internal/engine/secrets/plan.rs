//! Pure mapping from compose `secrets:`/`configs:` references to native-secret
//! plans. No daemon access, so the mapping is unit-testable; the create and
//! preflight side effects live in [`super`]'s `Engine` impl.

use crate::compose::types::{ComposeFile, Service, ServiceConfigRef, ServiceSecretRef};
use crate::error::{ComposeError, Result};

use super::super::staging;

/// Podman's hard limit on secret payload size (from `containers/common`): the
/// payload must be larger than 0 and strictly smaller than this many bytes.
pub(super) const MAX_SECRET_BYTES: usize = 512_000;

/// A planned native secret for a service: the Podman secret `source` to attach,
/// the in-container `target`, optional permissions, and — for inline
/// `content:`/`environment:` sources — the `payload` to create under `source`.
/// `external: true` references carry no payload (the secret must pre-exist).
pub(super) struct NativePlan {
	pub(super) source: String,
	pub(super) target: String,
	pub(super) mode: Option<u32>,
	pub(super) uid: Option<u32>,
	pub(super) gid: Option<u32>,
	pub(super) payload: Option<Vec<u8>>,
}

/// Where a secret/config's bytes come from once the compose def is resolved.
enum Source {
	/// `file:` — handled by the bind path, never a native secret.
	Bind,
	/// Inline `content:`/`environment:` — `(scoped podman name, payload bytes)`.
	Inline(String, Vec<u8>),
	/// `external: true` — name of the pre-existing podman secret.
	External(String),
}

/// Collect the native-secret plans for a service without touching the daemon. A
/// dangerous `mode:` (execute/setuid/setgid/sticky) is rejected here so a
/// hostile mode never reaches Podman.
pub(super) fn collect_native_plans(
	project: &str,
	service: &Service,
	file: &ComposeFile,
) -> Result<Vec<NativePlan>> {
	let mut plans = Vec::new();

	for secret_ref in &service.secrets {
		let (name, target_override, mode, uid, gid) = secret_ref_parts(secret_ref);
		if let Some(def) = file.secrets.get(&name) {
			let source = resolve_source(
				project,
				"secret",
				&name,
				def.content.as_deref(),
				def.environment.as_deref(),
				def.external == Some(true),
				def.name.as_deref(),
			)?;
			// A bare target name lands under /run/secrets/<name>, matching the
			// bind-mount default and the external-secret behaviour.
			push_plan(
				&mut plans,
				source,
				target_override.unwrap_or(name),
				mode,
				uid,
				gid,
			)?;
		}
	}

	for config_ref in &service.configs {
		let (name, target_override, mode, uid, gid) = config_ref_parts(config_ref);
		if let Some(def) = file.configs.get(&name) {
			let source = resolve_source(
				project,
				"config",
				&name,
				def.content.as_deref(),
				def.environment.as_deref(),
				def.external == Some(true),
				def.name.as_deref(),
			)?;
			// Configs default to an absolute container-root path, matching the
			// bind-mount config behaviour.
			let target = target_override.unwrap_or_else(|| format!("/{name}"));
			push_plan(&mut plans, source, target, mode, uid, gid)?;
		}
	}

	Ok(plans)
}

/// Resolve a secret/config definition to its native [`Source`]. `external`
/// wins (it may also carry a custom `name:`); otherwise inline `content:` or
/// `environment:` become a project-scoped native secret; anything else (a
/// `file:` source, or an empty def) is left to the bind path.
fn resolve_source(
	project: &str,
	kind: &str,
	name: &str,
	content: Option<&str>,
	environment: Option<&str>,
	external: bool,
	external_name: Option<&str>,
) -> Result<Source> {
	if external {
		return Ok(Source::External(external_name.unwrap_or(name).to_string()));
	}
	let is_inline = content.is_some() || environment.is_some();
	if is_inline && !staging::is_safe_project_name(name) {
		// The name becomes part of the project-scoped Podman secret name and a URL
		// query parameter, so require a bounded, well-formed identifier rather than
		// an arbitrary (possibly huge or control-laden) YAML key.
		return Err(ComposeError::Unsupported(format!(
			"{kind} name {name:?} must be ASCII alphanumeric/dash/underscore/dot, \
			 at most 128 chars, and not start with a dot"
		)));
	}
	if let Some(content) = content {
		return Ok(Source::Inline(
			scoped_name(project, kind, name),
			content.as_bytes().to_vec(),
		));
	}
	if let Some(env_var) = environment {
		let value = std::env::var(env_var).map_err(|_| {
			ComposeError::Unsupported(format!(
				"{kind} '{name}' references env var '{env_var}' which is not set"
			))
		})?;
		return Ok(Source::Inline(
			scoped_name(project, kind, name),
			value.into_bytes(),
		));
	}
	Ok(Source::Bind)
}

/// Append a [`NativePlan`] for a resolved source, dropping `file:` sources and
/// rejecting a dangerous `mode:` before the spec is built. `uid`/`gid` are
/// numeric in libpod, so a non-numeric value (a user/group name) is dropped to
/// the default rather than erroring.
fn push_plan(
	plans: &mut Vec<NativePlan>,
	source: Source,
	target: String,
	mode: Option<u32>,
	uid: Option<String>,
	gid: Option<String>,
) -> Result<()> {
	let (source, payload) = match source {
		Source::Bind => return Ok(()),
		Source::Inline(s, p) => (s, Some(p)),
		Source::External(s) => (s, None),
	};
	// Default to the Compose Specification's world-readable `0444` when no `mode:`
	// is given. A Podman-native secret otherwise mounts at `0000`, which a non-root
	// container user cannot read (only root reads it via DAC override), diverging
	// from docker-compose where the default is readable.
	let mode = mode.or(Some(0o444));
	if let Some(m) = mode {
		staging::reject_dangerous_secret_mode(m, &source)?;
	}
	plans.push(NativePlan {
		source,
		target,
		mode,
		uid: uid.and_then(|s| s.parse().ok()),
		gid: gid.and_then(|s| s.parse().ok()),
		payload,
	});
	Ok(())
}

/// Project-scoped Podman secret name for an inline secret/config, namespaced by
/// `kind` so a secret and a config sharing a compose name do not collide.
pub(super) fn scoped_name(project: &str, kind: &str, name: &str) -> String {
	format!("{project}_{kind}_{name}")
}

/// Reject a payload Podman would refuse (`len == 0` or `>= MAX_SECRET_BYTES`),
/// with a clearer message than the daemon's opaque 500.
pub(super) fn check_secret_size(name: &str, len: usize) -> Result<()> {
	if len == 0 || len >= MAX_SECRET_BYTES {
		return Err(ComposeError::Unsupported(format!(
			"secret '{name}' is {len} bytes; a Podman secret payload must be \
			 larger than 0 and smaller than {MAX_SECRET_BYTES} bytes"
		)));
	}
	Ok(())
}

/// Whether a secret/config def is an inline `content:`/`environment:` source —
/// i.e. one podup creates as a project-scoped native secret. `external:` wins
/// (it is never created by podup) and a bare `file:` source is a bind mount.
pub(super) fn is_inline_source(
	external: Option<bool>,
	content: Option<&str>,
	environment: Option<&str>,
) -> bool {
	external != Some(true) && (content.is_some() || environment.is_some())
}

/// `(name, target_override)` for a service secret/config reference.
pub(super) fn ref_name_target(source: &str, target: Option<&str>) -> (String, Option<String>) {
	(source.to_string(), target.map(str::to_string))
}

/// Decompose a secret reference into `(name, target, mode, uid, gid)`.
fn secret_ref_parts(
	r: &ServiceSecretRef,
) -> (
	String,
	Option<String>,
	Option<u32>,
	Option<String>,
	Option<String>,
) {
	match r {
		ServiceSecretRef::Short(s) => (s.clone(), None, None, None, None),
		ServiceSecretRef::Long {
			source,
			target,
			mode,
			uid,
			gid,
		} => (
			source.clone(),
			target.clone(),
			*mode,
			uid.clone(),
			gid.clone(),
		),
	}
}

/// Decompose a config reference into `(name, target, mode, uid, gid)`.
fn config_ref_parts(
	r: &ServiceConfigRef,
) -> (
	String,
	Option<String>,
	Option<u32>,
	Option<String>,
	Option<String>,
) {
	match r {
		ServiceConfigRef::Short(s) => (s.clone(), None, None, None, None),
		ServiceConfigRef::Long {
			source,
			target,
			mode,
			uid,
			gid,
		} => (
			source.clone(),
			target.clone(),
			*mode,
			uid.clone(),
			gid.clone(),
		),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn plans(yaml: &str) -> Vec<NativePlan> {
		let file = crate::compose::parse_str_raw(yaml).unwrap();
		collect_native_plans("proj", &file.services["web"], &file).unwrap()
	}

	#[test]
	fn file_secret_is_not_a_native_secret() {
		// A `file:` secret is a bind mount, never a native secret.
		let p = plans("services:\n  web:\n    image: nginx\n    secrets: [tok]\nsecrets:\n  tok:\n    file: ./tok.txt\n");
		assert!(p.is_empty());
	}

	#[test]
	fn inline_content_secret_is_scoped_native_with_payload() {
		// `content:` becomes a project-scoped native secret carrying the bytes;
		// the mount target defaults to the bare compose name (→ /run/secrets/tok).
		let p = plans("services:\n  web:\n    image: nginx\n    secrets: [tok]\nsecrets:\n  tok:\n    content: supersecret\n");
		assert_eq!(p.len(), 1);
		assert_eq!(p[0].source, "proj_secret_tok");
		assert_eq!(p[0].target, "tok");
		assert_eq!(p[0].payload.as_deref(), Some(b"supersecret".as_slice()));
	}

	#[test]
	fn inline_content_config_is_scoped_native_with_absolute_target() {
		// Configs default to an absolute container-root path.
		let p = plans("services:\n  web:\n    image: nginx\n    configs: [cfg]\nconfigs:\n  cfg:\n    content: key=value\n");
		assert_eq!(p.len(), 1);
		assert_eq!(p[0].source, "proj_config_cfg");
		assert_eq!(p[0].target, "/cfg");
		assert_eq!(p[0].payload.as_deref(), Some(b"key=value".as_slice()));
	}

	#[test]
	fn env_secret_payload_comes_from_environment() {
		temp_env::with_var("PODUP_TEST_SECRET", Some("env-value"), || {
			let p = plans("services:\n  web:\n    image: nginx\n    secrets: [tok]\nsecrets:\n  tok:\n    environment: PODUP_TEST_SECRET\n");
			assert_eq!(p.len(), 1);
			assert_eq!(p[0].source, "proj_secret_tok");
			assert_eq!(p[0].payload.as_deref(), Some(b"env-value".as_slice()));
		});
	}

	#[test]
	fn env_secret_missing_var_errors() {
		temp_env::with_var("PODUP_TEST_MISSING", None::<&str>, || {
			let file = crate::compose::parse_str_raw("services:\n  web:\n    image: nginx\n    secrets: [tok]\nsecrets:\n  tok:\n    environment: PODUP_TEST_MISSING\n").unwrap();
			assert!(collect_native_plans("proj", &file.services["web"], &file).is_err());
		});
	}

	#[test]
	fn external_secret_keeps_compose_name_unscoped_no_payload() {
		// An `external: true` secret points at a pre-existing podman secret: the
		// source equals the compose name (no project scoping) and carries no
		// payload. The mount filename defaults to the compose name.
		let p = plans("services:\n  web:\n    image: nginx\n    secrets: [tok]\nsecrets:\n  tok:\n    external: true\n");
		assert_eq!(p.len(), 1);
		assert_eq!(p[0].source, "tok");
		assert_eq!(p[0].target, "tok");
		assert!(p[0].payload.is_none());
	}

	#[test]
	fn external_secret_long_form_maps_source_target_and_perms() {
		// A long-form ref overrides the mount name, a custom top-level `name:` is
		// the real podman secret, and numeric uid/gid/mode pass through. `mode:` is
		// octal notation per the Compose Specification (leading-zero `0400`).
		let p = plans("services:\n  web:\n    image: nginx\n    secrets:\n      - source: tok\n        target: app_tok\n        uid: \"100\"\n        gid: \"101\"\n        mode: 0400\nsecrets:\n  tok:\n    external: true\n    name: real_tok\n");
		assert_eq!(p.len(), 1);
		assert_eq!(p[0].source, "real_tok");
		assert_eq!(p[0].target, "app_tok");
		assert_eq!(p[0].uid, Some(100));
		assert_eq!(p[0].gid, Some(101));
		assert_eq!(p[0].mode, Some(0o400));
	}

	#[test]
	fn external_config_becomes_native_with_absolute_default_target() {
		let p = plans("services:\n  web:\n    image: nginx\n    configs: [cfg]\nconfigs:\n  cfg:\n    external: true\n");
		assert_eq!(p.len(), 1);
		assert_eq!(p[0].source, "cfg");
		assert_eq!(p[0].target, "/cfg");
	}

	#[test]
	fn non_numeric_uid_drops_to_default() {
		// libpod secret uid/gid are numeric; a user/group name falls back to the
		// default rather than erroring.
		let p = plans("services:\n  web:\n    image: nginx\n    secrets:\n      - source: tok\n        uid: appuser\nsecrets:\n  tok:\n    external: true\n");
		assert_eq!(p.len(), 1);
		assert!(p[0].uid.is_none());
	}

	#[test]
	fn native_secret_rejects_setuid_mode() {
		// 0o4000 (= 2048) is setuid; refused before the spec reaches Podman.
		let file = crate::compose::parse_str_raw("services:\n  web:\n    image: nginx\n    secrets:\n      - source: tok\n        mode: 2048\nsecrets:\n  tok:\n    external: true\n").unwrap();
		assert!(collect_native_plans("proj", &file.services["web"], &file).is_err());
	}

	#[test]
	fn native_secret_rejects_execute_mode() {
		// 0o777 (= 511) sets execute bits; a secret holds data, never code.
		let file = crate::compose::parse_str_raw("services:\n  web:\n    image: nginx\n    secrets:\n      - source: tok\n        mode: 511\nsecrets:\n  tok:\n    external: true\n").unwrap();
		assert!(collect_native_plans("proj", &file.services["web"], &file).is_err());
	}

	#[test]
	fn native_config_rejects_setgid_mode() {
		// External configs share the mode guard. 0o2000 (= 1024) is setgid.
		let file = crate::compose::parse_str_raw("services:\n  web:\n    image: nginx\n    configs:\n      - source: cfg\n        mode: 1024\nconfigs:\n  cfg:\n    external: true\n").unwrap();
		assert!(collect_native_plans("proj", &file.services["web"], &file).is_err());
	}

	#[test]
	fn inline_secret_rejects_dangerous_mode() {
		// The mode guard also covers project-created inline secrets.
		let file = crate::compose::parse_str_raw("services:\n  web:\n    image: nginx\n    secrets:\n      - source: tok\n        mode: 511\nsecrets:\n  tok:\n    content: data\n").unwrap();
		assert!(collect_native_plans("proj", &file.services["web"], &file).is_err());
	}

	#[test]
	fn native_secret_allows_world_readable_mode() {
		// 0o444 (= 292) is the Podman/compose default for an in-container secret
		// and must be allowed (unlike the old shared-host staging path).
		let p = plans("services:\n  web:\n    image: nginx\n    secrets:\n      - source: tok\n        mode: 292\nsecrets:\n  tok:\n    external: true\n");
		assert_eq!(p[0].mode, Some(0o444));
	}

	#[test]
	fn empty_and_oversized_payloads_rejected() {
		assert!(check_secret_size("s", 0).is_err());
		assert!(check_secret_size("s", MAX_SECRET_BYTES).is_err());
		assert!(check_secret_size("s", MAX_SECRET_BYTES - 1).is_ok());
		assert!(check_secret_size("s", 1).is_ok());
	}

	#[test]
	fn inline_secret_with_unsafe_name_is_rejected() {
		// A path-traversal / control-laden key must not become a Podman secret name.
		let file = crate::compose::parse_str_raw(
			"services:\n  web:\n    image: x\n    secrets: ['../evil']\nsecrets:\n  '../evil':\n    content: data\n",
		)
		.unwrap();
		assert!(collect_native_plans("proj", &file.services["web"], &file).is_err());
	}

	#[test]
	fn native_secret_without_mode_defaults_to_0444() {
		// The Compose Specification default is world-readable 0444; a Podman-native
		// secret otherwise mounts at 0000 and a non-root container user can't read it.
		let p = plans("services:\n  web:\n    image: x\n    secrets: [tok]\nsecrets:\n  tok:\n    content: data\n");
		assert_eq!(p.len(), 1);
		assert_eq!(p[0].mode, Some(0o444));
	}

	#[test]
	fn secret_mode_leading_zero_is_octal() {
		// `0444` (leading-zero octal, the Compose Specification spelling) parses as
		// octal 0o444, not decimal 444 (which would fail) — issue #2.
		let p = plans("services:\n  web:\n    image: x\n    secrets:\n      - source: tok\n        mode: 0444\nsecrets:\n  tok:\n    content: data\n");
		assert_eq!(p[0].mode, Some(0o444));
	}

	#[test]
	fn is_inline_source_classifies_sources() {
		assert!(is_inline_source(None, Some("x"), None));
		assert!(is_inline_source(None, None, Some("VAR")));
		assert!(!is_inline_source(Some(true), Some("x"), None));
		assert!(!is_inline_source(None, None, None));
	}
}
