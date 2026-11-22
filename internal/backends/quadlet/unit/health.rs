//! Map a compose `healthcheck:` onto the Quadlet `Health*=` keys.

use crate::compose::types::{Command, Service};

use super::Section;

/// Map a compose `healthcheck:` onto the Quadlet `Health*=` keys. A disabled
/// healthcheck emits `HealthCmd=none`; otherwise the compose test (with any
/// leading `CMD`/`CMD-SHELL`/`NONE` sentinel stripped) and the timing fields
/// are rendered. Fields with no Quadlet/Podman equivalent push a warning into
/// `warnings` instead of being silently dropped. Returns whether the unit waits
/// for the healthcheck to pass (`Notify=healthy`).
pub(super) fn render_healthcheck(
	name: &str,
	service: &Service,
	container: &mut Section,
	warnings: &mut Vec<String>,
) -> bool {
	let Some(hc) = &service.healthcheck else {
		return false;
	};
	if hc.is_disabled() {
		container.add("HealthCmd", "none".to_string());
		return false;
	}
	let mut waits = false;
	if let Some(test) = &hc.test {
		let cmd = match test {
			Command::Shell(s) => s.clone(),
			Command::Exec(parts) => match parts.first().map(String::as_str) {
				// `CMD-SHELL` carries one string that is already a shell command.
				Some("CMD-SHELL") | Some("NONE") => parts[1..].join(" "),
				// `CMD` is an argument vector. Podman reads a `HealthCmd=` value that is
				// a JSON array as the exec form (measured on Podman 6.2.0,
				// 2026-11-22: `--health-cmd '["CMD","wget","-q","a b"]'` stores
				// `Test: ["CMD","wget","-q","a b"]`, where a plain string is stored as
				// `CMD-SHELL`), so the words reach the probe as they are and the image
				// needs no shell.
				Some("CMD") if parts.len() > 1 => serde_json::to_string(parts).unwrap_or_default(),
				Some("CMD") => String::new(),
				_ => parts.join(" "),
			},
		};
		if !cmd.is_empty() {
			container.add("HealthCmd", cmd);
			// Tell systemd the unit is not up until the healthcheck passes.
			// Quadlet renders this as `--sdnotify=healthy`, and without it
			// systemd calls the service started the moment the container is
			// created, so `After=`/`Requires=` order the *creation*, not the
			// readiness, and a dependant starts against a service that is not
			// serving yet.
			//
			// With the 6.2.0 generator the unit is `Type=notify` and `podman run`
			// gets `--sdnotify=healthy`, so systemd waits for the probe to pass
			// and a unit ordered `After=`/`Requires=` on this one starts once the
			// service is ready: that is `depends_on: service_healthy` being
			// honoured.
			//
			// Only emitted alongside a command. `--sdnotify=healthy` with
			// nothing to probe would leave the unit starting until
			// `TimeoutStartSec` expires.
			container.add("Notify", "healthy".to_string());
			waits = true;
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
	// The `x-podman-on-failure` extension. An invalid value is warned about
	// rather than emitted: generation has no error channel here, and writing an
	// unrecognised `HealthOnFailure=` would make Quadlet drop the whole unit at
	// daemon-reload, a far worse failure than the key being absent. The live
	// `up` path rejects the same value outright, where it can.
	match hc.podman_on_failure() {
		Ok(Some(action)) => container.add("HealthOnFailure", action.as_str().to_string()),
		Ok(None) => {}
		Err(e) => warnings.push(format!("{name}: {e}")),
	}
	if hc.start_interval.is_some() {
		// Compose `start_interval` (the probe interval during the start period)
		// has no Quadlet/Podman equivalent. The Quadlet `HealthStartupInterval=`
		// key drives Podman's separate *startup healthcheck* feature, which is a
		// no-op without a `HealthStartupCmd=`; on Podman 6.2.0 (2026-11-22)
		// `--health-start-interval` is only an alias of `--health-startup-interval`,
		// so it is no equivalent either. Skip it and warn rather than emit a key
		// that silently does nothing.
		warnings.push(format!(
			"{name}: healthcheck.start_interval has no Quadlet/Podman equivalent and is skipped"
		));
	}
	waits
}

#[cfg(test)]
#[path = "health_tests.rs"]
mod tests;
