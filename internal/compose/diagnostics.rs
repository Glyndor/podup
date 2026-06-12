//! Parse-time diagnostics.
//!
//! podup accepts the full compose-spec surface and, per the spec's
//! forward-compatibility rule, never treats an unknown key as a hard error.
//! The cost of that leniency is silent drops: a typo or an unmapped compose
//! feature would just vanish. This pass closes that gap by reporting every key
//! or field podup parses but cannot translate, so nothing is ignored without
//! the operator hearing about it — the same guarantee that lets podup absorb
//! future Docker/Podman compose additions gracefully.

use super::types::{BuildConfig, ComposeFile};

/// Collect a warning for every parsed-but-unsupported key or field in `file`.
/// Pure (no logging) so it can be unit-tested; the caller emits the messages.
pub(super) fn collect(file: &ComposeFile) -> Vec<String> {
	let mut out = Vec::new();
	unknown_top_level_keys(file, &mut out);
	unknown_service_keys(file, &mut out);
	ignored_service_fields(file, &mut out);
	ignored_build_fields(file, &mut out);
	ignored_network_fields(file, &mut out);
	out
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

/// Service keys that matched no known field. A likely typo — e.g.
/// `enviroment:` — is easy to miss when it just vanishes, so surface it.
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

/// Service fields that podup models but cannot honor on rootless Podman.
fn ignored_service_fields(file: &ComposeFile, out: &mut Vec<String>) {
	for (service, def) in &file.services {
		if def.cpu_count.is_some() {
			out.push(format!(
				"service '{service}': cpu_count is a Windows/Hyper-V control with no \
				 rootless Podman equivalent and is ignored"
			));
		}
		if def.cpu_percent.is_some() {
			out.push(format!(
				"service '{service}': cpu_percent is a Windows/Hyper-V control with no \
				 rootless Podman equivalent and is ignored"
			));
		}
	}
}

/// Build options that exist only in BuildKit/buildx and have no libpod
/// build-API mapping. Honored fields are left untouched.
fn ignored_build_fields(file: &ComposeFile, out: &mut Vec<String>) {
	for (service, def) in &file.services {
		let Some(BuildConfig::Config {
			privileged,
			ulimits,
			isolation,
			entitlements,
			provenance,
			sbom,
			..
		}) = &def.build
		else {
			continue;
		};
		let mut unmapped: Vec<&str> = Vec::new();
		if privileged.is_some() {
			unmapped.push("privileged");
		}
		if !ulimits.is_empty() {
			unmapped.push("ulimits");
		}
		if isolation.is_some() {
			unmapped.push("isolation");
		}
		if !entitlements.is_empty() {
			unmapped.push("entitlements");
		}
		if provenance.is_some() {
			unmapped.push("provenance");
		}
		if sbom.is_some() {
			unmapped.push("sbom");
		}
		for field in unmapped {
			out.push(format!(
				"service '{service}': build.{field} has no libpod build-API equivalent \
				 and is ignored"
			));
		}
	}
}

/// Network fields podup parses but does not forward to Podman.
fn ignored_network_fields(file: &ComposeFile, out: &mut Vec<String>) {
	for (name, cfg) in &file.networks {
		if let Some(c) = cfg {
			if c.enable_ipv4.is_some() {
				out.push(format!(
					"network '{name}': enable_ipv4 is not forwarded; Podman networks \
					 enable IPv4 by default and expose no toggle"
				));
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use crate::parse_str;

	fn diagnostics_for(yaml: &str) -> Vec<String> {
		let file = parse_str(yaml).unwrap();
		super::collect(&file)
	}

	#[test]
	fn warns_on_unknown_top_level_key_but_not_x_extension() {
		let msgs = diagnostics_for(
			"x-anchors: ok\nservies:\n  typo: 1\nservices:\n  web:\n    image: nginx\n",
		);
		assert!(
			msgs.iter()
				.any(|m| m.contains("unknown top-level key 'servies'")),
			"got: {msgs:?}"
		);
		assert!(!msgs.iter().any(|m| m.contains("x-anchors")));
	}

	#[test]
	fn warns_on_unknown_service_key_but_not_x_extension() {
		let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    enviroment:\n      A: 1\n    x-meta: ok\n",
		);
		assert!(msgs.iter().any(|m| m.contains("unknown key 'enviroment'")));
		assert!(!msgs.iter().any(|m| m.contains("x-meta")));
	}

	#[test]
	fn warns_on_windows_only_cpu_fields() {
		let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    cpu_count: 2\n    cpu_percent: 50\n",
		);
		assert!(msgs.iter().any(|m| m.contains("cpu_count")));
		assert!(msgs.iter().any(|m| m.contains("cpu_percent")));
	}

	#[test]
	fn warns_on_unmapped_build_fields() {
		let msgs = diagnostics_for(
			"services:\n  web:\n    build:\n      context: .\n      privileged: true\n      isolation: chroot\n",
		);
		assert!(msgs.iter().any(|m| m.contains("build.privileged")));
		assert!(msgs.iter().any(|m| m.contains("build.isolation")));
	}

	#[test]
	fn warns_on_network_enable_ipv4() {
		let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\nnetworks:\n  net:\n    enable_ipv4: false\n",
		);
		assert!(msgs.iter().any(|m| m.contains("enable_ipv4")));
	}

	#[test]
	fn clean_file_produces_no_diagnostics() {
		let msgs = diagnostics_for("services:\n  web:\n    image: nginx\n    cpu_shares: 512\n");
		assert!(msgs.is_empty(), "unexpected diagnostics: {msgs:?}");
	}
}
