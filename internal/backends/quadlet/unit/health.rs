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
			Command::Exec(parts) => match parts.first().map(String::as_str) {
				// `CMD-SHELL` carries one string that is already a shell command.
				Some("CMD-SHELL") | Some("NONE") => parts[1..].join(" "),
				// `CMD` is an argument vector, but `HealthCmd=` is read by Podman as
				// a shell command line, so each word is quoted: joined with plain
				// spaces, `["CMD", "sh", "-c", "wget -q x || exit 1"]` ran `wget`
				// with no argument and then `exit 1`.
				Some("CMD") => {
					// Podman 5 reads `HealthCmd=` only as a shell command line, so the
					// exec form cannot be kept: the probe runs through `/bin/sh -c`,
					// where `up` runs it directly. In an image without a shell
					// (distroless, scratch) it then fails every time and a
					// `Notify=healthy` unit never starts.
					warnings.push(format!(
						"{name}: healthcheck test [\"CMD\", ...] runs through /bin/sh in the \
						 Quadlet unit (HealthCmd= is a shell command line); the image needs a shell"
					));
					parts[1..]
						.iter()
						.map(|word| shell_word(word))
						.collect::<Vec<_>>()
						.join(" ")
				}
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
			// Measured on podman 5.7.0 with a probe that takes ten seconds to
			// pass: `systemctl start` on the unit itself returned after 11s
			// rather than immediately, and a second unit with
			// `After=`/`Requires=` on it started 10s in. That second number is
			// the one that matters: it is `depends_on: service_healthy`
			// actually being honoured.
			//
			// Only emitted alongside a command. `--sdnotify=healthy` with
			// nothing to probe would leave the unit starting until
			// `TimeoutStartSec` expires.
			container.add("Notify", "healthy".to_string());
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
		// no-op without a `HealthStartupCmd=`; Podman 5.x exposes no
		// `--health-start-interval`. Skip it and warn rather than emit a key that
		// silently does nothing.
		warnings.push(format!(
			"{name}: healthcheck.start_interval has no Quadlet/Podman equivalent and is skipped"
		));
	}
}

#[cfg(test)]
#[path = "health_tests.rs"]
mod tests;

/// One word of a command line, quoted for `sh` only when it needs it.
fn shell_word(word: &str) -> String {
	let plain = !word.is_empty()
		&& word.chars().all(|c| {
			c.is_ascii_alphanumeric()
				|| matches!(c, '-' | '_' | '.' | '/' | ':' | '=' | ',' | '@' | '%' | '+')
		});
	if plain {
		word.to_string()
	} else {
		format!("'{}'", word.replace('\'', "'\\''"))
	}
}
