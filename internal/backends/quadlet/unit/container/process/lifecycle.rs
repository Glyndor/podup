//! How the unit stops and restarts the container: the stop signal and timeout,
//! and the restart policy.

use crate::compose::types::{RestartPolicy, Service};
use crate::size::parse_duration_secs_rounded_up;

use super::super::{quote_podman_arg_value, render_restart, Section};

/// Emit the stop signal and the stop timeout of the container.
pub(in super::super) fn emit_stop(service: &Service, container: &mut Section) {
	if let Some(signal) = &service.stop_signal {
		// An empty signal (`stop_signal: ""`) would render as
		// `--stop-signal=`, which podman rejects. The native
		// `StopSignal=` key omits empty values, so the podman-args
		// route has to skip them too.
		if signal.is_empty() {
			// Skip without warning: an empty `stop_signal:` is a no-op
			// the user almost certainly did not intend, but the same
			// input was already a no-op via the native key, and the
			// podman-args route is the new code path; matching the
			// old behaviour silently is the smallest change.
		} else {
			// `StopSignal=` was added to Quadlet in Podman 5.2.0. The
			// supported floor is 5.0, so route the signal through
			// `PodmanArgs=` as `--stop-signal`, like the other
			// escape-hatch flags. Quoted per #1734: a hostile value
			// carrying whitespace must not become two argv elements.
			container.add(
				"PodmanArgs",
				format!("--stop-signal={}", quote_podman_arg_value(signal)),
			);
		}
	}
	if let Some(grace) = &service.stop_grace_period {
		// Rounded up: a grace period of half a second is a grace period, not an
		// instant kill.
		if let Some(secs) = parse_duration_secs_rounded_up(grace) {
			container.add("StopTimeout", secs.to_string());
		}
	}
}

/// Emit the `[Service]` section: `Restart=` plus the start limits that go with
/// it, from either `restart:` or `deploy.restart_policy`.
pub(in super::super) fn emit_restart(service: &Service, svc: &mut Section) {
	if let Some(restart) = &service.restart {
		svc.add("Restart", render_restart(restart));
		if let RestartPolicy::OnFailure {
			max_attempts: Some(n),
		} = restart
		{
			svc.add("StartLimitBurst", n.to_string());
		}
	} else if let Some(rp) = service
		.deploy
		.as_ref()
		.and_then(|d| d.restart_policy.as_ref())
	{
		// `deploy.restart_policy` is the modern equivalent of the service-level
		// `restart:` string; its `condition` maps onto the systemd `Restart=`
		// values and `max_attempts`/`window` onto the start-limit window.
		let restart = match rp.condition.as_deref() {
			Some("none") => "no",
			Some("on-failure") => "on-failure",
			// "any" (the compose default) and any unknown value restart always.
			_ => "always",
		};
		svc.add("Restart", restart.to_string());
		if let Some(n) = rp.max_attempts {
			svc.add("StartLimitBurst", n.to_string());
			if let Some(secs) = rp
				.window
				.as_deref()
				.and_then(parse_duration_secs_rounded_up)
			{
				svc.add("StartLimitIntervalSec", secs.to_string());
			}
		}
	}
}

/// systemd gives a unit 90 seconds to start and to stop. Quadlet sets neither
/// from the container keys, so a `stop_grace_period` past that was cut short by
/// SIGKILL, and a `Notify=healthy` unit whose healthcheck may take longer to
/// pass was marked failed while `up` would still be waiting. The budget is
/// written only when it is past the default.
pub(in super::super) fn emit_timeouts(
	service: &Service,
	waits_for_health: bool,
	svc: &mut Section,
) {
	const SYSTEMD_DEFAULT: u64 = 90;
	// Margin for `podman rm` and the conmon round trips around the container.
	const MARGIN: u64 = 30;
	let secs = |value: Option<&String>, default: u64| {
		value
			.and_then(|v| parse_duration_secs_rounded_up(v))
			.unwrap_or(default)
	};
	if waits_for_health {
		if let Some(hc) = &service.healthcheck {
			// The longest a healthcheck takes to be declared unhealthy: the start
			// period, then every retry running for its interval and timeout
			// (compose defaults: 30s, 30s, 3).
			let retries = u64::from(hc.retries.unwrap_or(3));
			let budget = secs(hc.start_period.as_ref(), 0)
				.saturating_add(retries.saturating_mul(
					secs(hc.interval.as_ref(), 30).saturating_add(secs(hc.timeout.as_ref(), 30)),
				))
				.saturating_add(MARGIN);
			if budget > SYSTEMD_DEFAULT {
				svc.add("TimeoutStartSec", budget.to_string());
			}
		}
	}
	if let Some(grace) = service
		.stop_grace_period
		.as_deref()
		.and_then(parse_duration_secs_rounded_up)
	{
		let budget = grace.saturating_add(MARGIN);
		if budget > SYSTEMD_DEFAULT {
			svc.add("TimeoutStopSec", budget.to_string());
		}
	}
}
