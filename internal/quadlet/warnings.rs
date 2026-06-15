//! Report compose fields that are set but have no Quadlet mapping.

use crate::compose::types::Service;

/// Warn for fields that are set but have no Quadlet mapping, so the operator
/// knows the generated unit is incomplete rather than discovering it at run
/// time.
pub(super) fn collect_warnings(name: &str, service: &Service, warnings: &mut Vec<String>) {
	let mut warn = |field: &str, detail: &str| {
		warnings.push(format!("{name}: {field} {detail}"));
	};
	if service.build.is_some() {
		warn(
			"build",
			"has no Quadlet equivalent; build the image first and set `image`",
		);
	}
	let replicas = service
		.scale
		.or(service.deploy.as_ref().and_then(|d| d.replicas));
	if replicas.is_some_and(|r| r > 1) {
		warn(
			"scale/replicas",
			"is ignored; Quadlet emits a single container per service",
		);
	}
	if !service.configs.is_empty() {
		warn("configs", "have no Quadlet equivalent and are skipped");
	}
	if !service.volumes_from.is_empty() {
		warn("volumes_from", "has no Quadlet equivalent and is skipped");
	}
	// `network_mode: host` maps to `Network=host`; other modes have no key.
	if service.network_mode.as_deref().is_some_and(|m| m != "host") {
		warn(
			"network_mode",
			"is not mapped (only `host` is supported); use networks instead",
		);
	}
	if !service.profiles.is_empty() {
		warn("profiles", "have no Quadlet equivalent and are ignored");
	}
	if service.privileged == Some(true) {
		warn(
			"privileged",
			"is not mapped; add PodmanArgs manually if required",
		);
	}
}

#[cfg(test)]
mod tests {
	use crate::parse_str;
	use crate::quadlet::generate;

	#[test]
	fn warns_for_every_unmapped_field() {
		let yaml = r#"
services:
  everything:
    image: app:1.0
    build: .
    scale: 3
    privileged: true
    network_mode: "container:other"
    volumes_from:
      - other
    profiles:
      - debug
    healthcheck:
      test: ["CMD", "true"]
    secrets:
      - my_secret
    configs:
      - my_config
secrets:
  my_secret:
    file: ./s.txt
configs:
  my_config:
    file: ./c.txt
"#;
		let file = parse_str(yaml).unwrap();
		let warnings = generate(&file, "proj").warnings;
		let joined = warnings.join("\n");

		for field in [
			"build",
			"scale/replicas",
			"configs",
			"volumes_from",
			"network_mode",
			"profiles",
			"privileged",
		] {
			assert!(
				joined.contains(field),
				"missing warning for {field}; got:\n{joined}"
			);
		}
		// secrets are now mapped to Secret=, so they must NOT warn.
		assert!(
			!joined.contains("secrets"),
			"secrets should be mapped, not warned; got:\n{joined}"
		);
	}

	#[test]
	fn clean_service_warns_about_nothing() {
		let yaml = r#"
services:
  web:
    image: nginx:1.27
"#;
		let file = parse_str(yaml).unwrap();
		assert!(generate(&file, "proj").warnings.is_empty());
	}
}
