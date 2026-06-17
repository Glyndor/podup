//! Map a compose `healthcheck:` onto the Quadlet `Health*=` keys.

use crate::compose::types::{Command, Service};

use super::Section;

/// Map a compose `healthcheck:` onto the Quadlet `Health*=` keys. A disabled
/// healthcheck emits `HealthCmd=none`; otherwise the compose test (with any
/// leading `CMD`/`CMD-SHELL`/`NONE` sentinel stripped) and the timing fields
/// are rendered.
pub(super) fn render_healthcheck(service: &Service, container: &mut Section) {
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
	if let Some(v) = &hc.start_interval {
		container.add("HealthStartupInterval", v.clone());
	}
}
