//! Map a compose `healthcheck:` onto the Quadlet `Health*=` keys.

use crate::compose::types::{Command, Service};

use super::Section;

/// Map a compose `healthcheck:` onto the Quadlet `Health*=` keys. A disabled
/// healthcheck emits `HealthCmd=none`; otherwise the compose test (with any
/// leading `CMD`/`CMD-SHELL`/`NONE` sentinel stripped) and the timing fields
/// are rendered. Fields with no Quadlet/Podman equivalent push a warning into
/// `warnings` instead of being silently dropped.
pub(super) fn render_healthcheck(
	name: &str,
	service: &Service,
	container: &mut Section,
	warnings: &mut Vec<String>,
) {
	let Some(hc) = &service.healthcheck else {
		return;
	};
	if hc.is_disabled() {
		container.add("HealthCmd", "none".to_string());
		return;
	}
	if let Some(test) = &hc.test {
		let cmd = match test {
			Command::Shell(s) => s.clone(),
			Command::Exec(parts) => {
				let body = match parts.first().map(String::as_str) {
					Some("CMD") | Some("CMD-SHELL") | Some("NONE") => &parts[1..],
					_ => &parts[..],
				};
				body.join(" ")
			}
		};
		if !cmd.is_empty() {
			container.add("HealthCmd", cmd);
		}
	}
	if let Some(v) = &hc.interval {
		container.add("HealthInterval", v.clone());
	}
	if let Some(v) = &hc.timeout {
		container.add("HealthTimeout", v.clone());
	}
	if let Some(v) = hc.retries {
		container.add("HealthRetries", v.to_string());
	}
	if let Some(v) = &hc.start_period {
		container.add("HealthStartPeriod", v.clone());
	}
	if hc.start_interval.is_some() {
		// Compose `start_interval` (the probe interval during the start period)
		// has no Quadlet/Podman equivalent. The Quadlet `HealthStartupInterval=`
		// key drives Podman's separate *startup healthcheck* feature, which is a
		// no-op without a `HealthStartupCmd=`; Podman 5.x exposes no
		// `--health-start-interval`. Skip it and warn rather than emit a key that
		// silently does nothing.
		warnings.push(format!(
			"{name}: healthcheck.start_interval has no Quadlet/Podman equivalent and is skipped"
		));
	}
}

#[cfg(test)]
mod tests {
	use super::{render_healthcheck, Section};
	use crate::compose::types::Service;

	/// Render a `[Container]` section for a service parsed from `yaml`,
	/// returning `(rendered_body, warnings)`.
	fn render(yaml: &str) -> (String, Vec<String>) {
		let service: Service = serde_yaml::from_str(yaml).expect("service parses");
		let mut container = Section::new("Container");
		let mut warnings = Vec::new();
		render_healthcheck("web", &service, &mut container, &mut warnings);
		(container.render(), warnings)
	}

	#[test]
	fn no_healthcheck_emits_nothing() {
		let (body, warnings) = render("image: x\n");
		assert_eq!(body, "[Container]\n");
		assert!(warnings.is_empty());
	}

	#[test]
	fn disabled_healthcheck_emits_none() {
		let (body, warnings) = render("image: x\nhealthcheck:\n  disable: true\n");
		assert!(body.contains("HealthCmd=none"));
		// A disabled check short-circuits — no timing keys follow.
		assert!(!body.contains("HealthInterval"));
		assert!(warnings.is_empty());
	}

	#[test]
	fn exec_test_strips_cmd_sentinel_and_renders_timing() {
		let (body, warnings) = render(
			"image: x\nhealthcheck:\n  test: [\"CMD\", \"curl\", \"-f\", \"http://localhost\"]\n  \
			 interval: 30s\n  timeout: 5s\n  retries: 3\n  start_period: 10s\n",
		);
		// The CMD sentinel is dropped from the rendered command.
		assert!(body.contains("HealthCmd=curl -f http://localhost"));
		assert!(body.contains("HealthInterval=30s"));
		assert!(body.contains("HealthTimeout=5s"));
		assert!(body.contains("HealthRetries=3"));
		assert!(body.contains("HealthStartPeriod=10s"));
		assert!(warnings.is_empty());
	}

	#[test]
	fn shell_test_renders_as_is() {
		let (body, _) = render("image: x\nhealthcheck:\n  test: curl -f http://localhost\n");
		assert!(body.contains("HealthCmd=curl -f http://localhost"));
	}

	#[test]
	fn exec_test_without_sentinel_keeps_all_parts() {
		// An exec array whose first element is not CMD/CMD-SHELL/NONE is rendered
		// verbatim (no sentinel stripped).
		let (body, _) = render("image: x\nhealthcheck:\n  test: [\"pg_isready\", \"-q\"]\n");
		assert!(body.contains("HealthCmd=pg_isready -q"));
	}

	#[test]
	fn start_interval_is_skipped_with_warning() {
		let (body, warnings) =
			render("image: x\nhealthcheck:\n  test: [\"CMD\", \"true\"]\n  start_interval: 5s\n");
		assert!(body.contains("HealthCmd=true"));
		// No key is emitted for start_interval, but the user is warned.
		assert_eq!(warnings.len(), 1);
		assert!(warnings[0].contains("start_interval"));
	}
}
