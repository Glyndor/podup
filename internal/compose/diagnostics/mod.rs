//! Parse-time diagnostics.
//!
//! podup accepts the full compose-spec surface and, per the spec's
//! forward-compatibility rule, never treats an unknown key as a hard error.
//! The cost of that leniency is silent drops: a typo or an unmapped compose
//! feature would just vanish. This pass closes that gap by reporting every key
//! or field podup parses but cannot translate, so nothing is ignored without
//! the operator hearing about it, the same guarantee that lets podup absorb
//! future Docker/Podman compose additions gracefully.

use std::cell::Cell;

use super::types::ComposeFile;

mod ignored_fields;
mod nested_raw;
pub(crate) use ignored_fields::ports_published_on_all_interfaces;
pub(crate) use ignored_fields::ports_published_on_wildcard;
use ignored_fields::{
	ignored_build_fields, ignored_models, ignored_network_fields, ignored_port_fields,
	ignored_restart_policy_fields, ignored_secret_config_drivers, ignored_service_fields,
	ignored_service_network_fields, ignored_volume_mount_fields, port_published_on_all_interfaces,
};
pub(super) use nested_raw::collect_raw_nested_warnings;

thread_local! {
	/// CLI gate that suppresses the parse-time "port ... is published on every
	/// interface" warning while set. The warning is host-binding (publishing on
	/// 0.0.0.0 is), so `--no-warn` already promises to cover it on the live
	/// engine paths; read-only commands (`ps`/`logs`/`port`/`top`) skip it
	/// unconditionally because there is no live action to confirm. `config`
	/// deliberately does not install the guard so its surface keeps the
	/// warning. The flag is read once per emitted warning by
	/// [`emit_diagnostic`] inside this module; the parse path does not have a
	/// parameter for it because the warning surface is also reached from
	/// library consumers (`parse_files_with_env_files`) that have no
	/// command-level context to pass.
	static SUPPRESS_PORT_EXPOSURE_WARNING: Cell<bool> = const { Cell::new(false) };
}

/// RAII guard that turns the parse-time port-exposure warning off for the
/// current thread, restoring the previous value on drop. The CLI driver
/// installs this around the parse step of the commands that opt out of the
/// warning; nested guards compose (each one restores its predecessor's
/// value), the same way the Quadlet path's [`crate::quadlet::NoWarnGuard`]
/// does, so a future parse nested inside a live one keeps the inner
/// setting instead of leaking the outer one.
pub struct SuppressPortExposureGuard {
	prev: bool,
}

impl SuppressPortExposureGuard {
	/// Activate the suppression for the current thread until the guard is
	/// dropped. Mirrors [`crate::quadlet::NoWarnGuard::new`]; embedders that
	/// drive the parse path directly (helmly-agent does for Quadlet) can use
	/// the same guard for the same effect.
	pub fn new() -> Self {
		let prev = SUPPRESS_PORT_EXPOSURE_WARNING.with(|c| c.get());
		SUPPRESS_PORT_EXPOSURE_WARNING.with(|c| c.set(true));
		Self { prev }
	}
}

impl Default for SuppressPortExposureGuard {
	fn default() -> Self {
		Self::new()
	}
}

impl Drop for SuppressPortExposureGuard {
	fn drop(&mut self) {
		SUPPRESS_PORT_EXPOSURE_WARNING.with(|c| c.set(self.prev));
	}
}

/// Whether the parse-time port-exposure warning is currently suppressed on
/// this thread. Read by [`emit_diagnostic`] before forwarding a warning to
/// `tracing::warn!`. Pure on the thread-local state.
fn port_exposure_suppressed() -> bool {
	SUPPRESS_PORT_EXPOSURE_WARNING.with(|c| c.get())
}

/// Identify the parse-time port-exposure warning. The text is matched on a
/// stable substring (`"is published on every interface"`) so the gate stays
/// correct if the warning is reworded to use a different service-name
/// prefix or a different port-binding suggestion, and so other warnings
/// are never accidentally silenced.
fn is_port_exposure_warning(msg: &str) -> bool {
	msg.contains("is published on every interface")
}

/// Emit one diagnostic warning, honouring the parse-time gate for the
/// port-exposure message. All other warnings are emitted unconditionally
/// to keep the existing behaviour for every other category.
pub(super) fn emit_diagnostic(msg: &str) {
	if is_port_exposure_warning(msg) && port_exposure_suppressed() {
		return;
	}
	tracing::warn!("{msg}");
}

/// Collect a warning for every parsed-but-unsupported key or field in `file`.
/// Pure (no logging) so it can be unit-tested; the caller emits the messages.
pub(super) fn collect(file: &ComposeFile) -> Vec<String> {
	let mut out = Vec::new();
	unknown_top_level_keys(file, &mut out);
	unknown_service_keys(file, &mut out);
	nested_unknown_keys(file, &mut out);
	ignored_service_fields(file, &mut out);
	ignored_port_fields(file, &mut out);
	port_published_on_all_interfaces(file, &mut out);
	ignored_volume_mount_fields(file, &mut out);
	ignored_build_fields(file, &mut out);
	ignored_network_fields(file, &mut out);
	ignored_service_network_fields(file, &mut out);
	ignored_secret_config_drivers(file, &mut out);
	ignored_models(file, &mut out);
	ignored_restart_policy_fields(file, &mut out);
	out
}

/// Push a warning for each non-`x-` key captured at a nested level.
fn push_unknown(
	context: &str,
	unknown: &indexmap::IndexMap<String, serde_yaml::Value>,
	out: &mut Vec<String>,
) {
	for key in unknown.keys() {
		if key.starts_with("x-") {
			continue;
		}
		out.push(format!(
			"{context}: unknown key '{key}' is ignored \
			 (check for a typo or an unsupported compose feature)"
		));
	}
}

/// Unknown keys captured inside service sub-objects and top-level network /
/// volume definitions. Together with the service- and top-level passes this
/// means a typo or an unmapped future field at ANY modeled level is surfaced
/// rather than silently dropped.
fn nested_unknown_keys(file: &ComposeFile, out: &mut Vec<String>) {
	for (service, def) in &file.services {
		if let Some(hc) = &def.healthcheck {
			push_unknown(
				&format!("service '{service}' healthcheck"),
				&hc.unknown,
				out,
			);
		}
		if let Some(deploy) = &def.deploy {
			push_unknown(&format!("service '{service}' deploy"), &deploy.unknown, out);
		}
		if let Some(develop) = &def.develop {
			for (i, rule) in develop.watch.iter().enumerate() {
				push_unknown(
					&format!("service '{service}' develop.watch[{i}]"),
					&rule.unknown,
					out,
				);
			}
		}
		if let Some(cred) = &def.credential_spec {
			push_unknown(
				&format!("service '{service}' credential_spec"),
				&cred.unknown,
				out,
			);
		}
		if let Some(provider) = &def.provider {
			push_unknown(
				&format!("service '{service}' provider"),
				&provider.unknown,
				out,
			);
		}
	}
	for (name, model) in &file.models {
		push_unknown(&format!("model '{name}'"), &model.unknown, out);
	}
	for (name, cfg) in &file.networks {
		if let Some(c) = cfg {
			push_unknown(&format!("network '{name}'"), &c.unknown, out);
			if let Some(ipam) = &c.ipam {
				push_unknown(&format!("network '{name}' ipam"), &ipam.unknown, out);
			}
		}
	}
	for (name, cfg) in &file.volumes {
		if let Some(c) = cfg {
			push_unknown(&format!("volume '{name}'"), &c.unknown, out);
		}
	}
}

/// Top-level keys that matched no known field (captured in `extensions`).
/// `x-*` extension keys are allowed by the spec and skipped.
fn unknown_top_level_keys(file: &ComposeFile, out: &mut Vec<String>) {
	for key in file.extensions.keys() {
		if key.starts_with("x-") {
			continue;
		}
		out.push(format!(
			"unknown top-level key '{key}' is ignored \
			 (check for a typo or an unsupported compose feature)"
		));
	}
}

/// Service keys that matched no known field. A likely typo, e.g.
/// `enviroment:`, is easy to miss when it just vanishes, so surface it.
fn unknown_service_keys(file: &ComposeFile, out: &mut Vec<String>) {
	for (service, def) in &file.services {
		for key in def.unknown.keys() {
			if key.starts_with("x-") {
				continue;
			}
			out.push(format!(
				"service '{service}': unknown key '{key}' is ignored \
				 (check for a typo or an unsupported compose feature)"
			));
		}
	}
}

#[cfg(test)]
mod tests;
