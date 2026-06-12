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
	if service.healthcheck.is_some() {
		warn("healthcheck", "is not yet mapped to HealthCmd directives");
	}
	if !service.secrets.is_empty() {
		warn(
			"secrets",
			"are not yet mapped to Quadlet Secret= directives",
		);
	}
	if !service.configs.is_empty() {
		warn("configs", "have no Quadlet equivalent and are skipped");
	}
	if !service.volumes_from.is_empty() {
		warn("volumes_from", "has no Quadlet equivalent and is skipped");
	}
	if service.network_mode.is_some() {
		warn("network_mode", "is not mapped; use networks instead");
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
