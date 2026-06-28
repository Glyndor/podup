//! Health and completion polling for service dependency ordering.
//!
//! [`Engine::wait_healthy`] polls until the container reports `healthy` (used when
//! a dependent service declares `condition: service_healthy`).
//! [`Engine::wait_completed`] polls until the container exits with code 0 (used for
//! `condition: service_completed_successfully`).

use std::time::Duration;

use serde::Deserialize;

use crate::compose::types::{ComposeFile, Service};
use crate::error::{ComposeError, Result};
use crate::libpod::types::container::ContainerInspect;
use crate::libpod::API_PREFIX;

use super::Engine;

/// Response from `GET {API_PREFIX}/containers/{name}/healthcheck`, which *runs*
/// the container's healthcheck on demand and reports the resulting status.
#[derive(Deserialize)]
struct HealthCheckRun {
	#[serde(rename = "Status")]
	status: Option<String>,
}

/// Per-poll verdict while waiting for `service_healthy`.
enum HealthVerdict {
	/// The runtime reports the container as `healthy`.
	Healthy,
	/// The container has no effective healthcheck, so `healthy` is unreachable;
	/// treat the dependency as satisfied rather than blocking until timeout.
	NoHealthcheck,
	/// The container has already exited non-zero. It can never become healthy and
	/// the service failed to start, so the wait must fail rather than report the
	/// dependency satisfied. Carries the exit code.
	Failed(i64),
	/// Not healthy yet — keep polling.
	Pending,
}

/// Classify a container inspect while waiting for `service_healthy`.
///
/// Pure decision logic for [`Engine::wait_healthy`], split out so the gating
/// behaviour can be unit-tested without a live Podman socket.
fn classify_health(info: &ContainerInspect) -> HealthVerdict {
	if let Some(state) = &info.state {
		if let Some(health) = &state.health {
			if health.status.as_deref() == Some("healthy") {
				return HealthVerdict::Healthy;
			}
		}
		// A container that has already exited can never become healthy. If it
		// exited non-zero the service failed to start, so fail the wait instead of
		// reporting it satisfied (which would mask the failure in `up --wait`/CI).
		// An exit code of 0 is a one-shot that completed; let it fall through.
		if state.status.as_deref() == Some("exited") {
			let code = state.exit_code.unwrap_or(0);
			if code != 0 {
				return HealthVerdict::Failed(code);
			}
		}
	}
	if !info
		.config
		.as_ref()
		.map(|c| c.has_healthcheck())
		.unwrap_or(false)
	{
		return HealthVerdict::NoHealthcheck;
	}
	HealthVerdict::Pending
}

/// Compute the `(poll_interval_secs, iterations)` for [`Engine::wait_healthy`]
/// from a healthcheck's `interval`/`start_period`/`retries`. Poll at `interval`
/// when given (>=1s) else 2s; run `retries` (default 30) probes plus enough
/// extra probes to span `start_period`. Pure so the timing can be unit-tested.
fn health_poll_plan(
	interval: Option<&str>,
	start_period: Option<&str>,
	retries: Option<u32>,
) -> (u64, u64) {
	let poll_secs = interval
		.and_then(crate::size::parse_duration_secs)
		.filter(|s| *s >= 1)
		.unwrap_or(2);
	let start_secs = start_period
		.and_then(crate::size::parse_duration_secs)
		.unwrap_or(0);
	let iterations = retries.unwrap_or(30) as u64 + start_secs / poll_secs;
	(poll_secs, iterations)
}

/// Poll iterations to actually run, given the healthcheck poll-plan and an
/// optional `--wait-timeout`.
///
/// `--wait-timeout` must be able to *extend* the wait, not merely cap it: a
/// healthcheck with a short `interval × retries` budget would otherwise time
/// out long before a generous `--wait-timeout` elapsed. So when a wait-timeout
/// is given we run at least enough iterations to cover it (plus a small margin),
/// letting the outer `--wait-timeout` deadline — not the poll plan — decide when
/// to give up. Without a wait-timeout the poll plan governs unchanged. Pure so
/// the budget arithmetic is unit-tested without a live socket.
fn effective_iterations(poll_secs: u64, plan_iters: u64, wait_timeout: Option<Duration>) -> u64 {
	match wait_timeout {
		Some(wt) => {
			let poll = poll_secs.max(1);
			// +2 so the inner loop outlasts the outer deadline, ensuring an
			// exhausted wait surfaces as the `--wait-timeout` error rather than a
			// spurious health-check-timeout that fired one poll early.
			let wt_iters = wt.as_secs().div_ceil(poll) + 2;
			plan_iters.max(wt_iters)
		}
		None => plan_iters,
	}
}

impl Engine {
	/// Wait until every targeted service's first replica is healthy (`up
	/// --wait`). A service with no effective healthcheck is treated as ready
	/// once started. All services when `target_services` is empty.
	pub async fn wait_services_healthy(
		&self,
		file: &ComposeFile,
		target_services: &[String],
	) -> Result<()> {
		self.wait_services_healthy_within(file, target_services, None)
			.await
	}

	/// As [`wait_services_healthy`](Self::wait_services_healthy), but a
	/// `Some(wait_timeout)` extends each service's poll budget to cover the
	/// supplied `--wait-timeout` (rather than only capping it). The caller still
	/// wraps the whole wait in a hard `--wait-timeout` deadline, which becomes the
	/// authoritative limit; this just stops the per-service poll plan from giving
	/// up early and reporting a misleading health-check timeout.
	pub async fn wait_services_healthy_within(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		wait_timeout: Option<Duration>,
	) -> Result<()> {
		// Poll services concurrently: each `wait_healthy` is its own poll loop, so
		// total `--wait` latency is the slowest service, not the sum of all.
		let waits = file
			.services
			.iter()
			.filter(|(name, _)| {
				target_services.is_empty() || target_services.iter().any(|t| t == *name)
			})
			.map(|(name, service)| {
				let container = self.first_replica_name(name, service);
				async move { self.wait_healthy(&container, service, wait_timeout).await }
			});
		futures_util::future::try_join_all(waits).await?;
		Ok(())
	}

	/// Poll a container until its health status is `healthy` or timeout.
	///
	/// Polls at the compose `healthcheck.interval` (default 2 s) for
	/// `healthcheck.retries` (default 30) probes, plus extra probes covering
	/// `healthcheck.start_period` so a slow-starting service is not timed out early.
	///
	/// The wait is driven by the container's *effective* healthcheck reported by
	/// the runtime, so healthchecks inherited from the image count too — not just
	/// those declared in compose. If the container has no effective healthcheck at
	/// all (none in the image or compose), it can never report `healthy`, so the
	/// wait short-circuits as satisfied rather than blocking until timeout.
	pub(super) async fn wait_healthy(
		&self,
		container_name: &str,
		service: &Service,
		wait_timeout: Option<Duration>,
	) -> Result<()> {
		let hc = service.healthcheck.as_ref();
		let (poll_secs, plan_iters) = health_poll_plan(
			hc.and_then(|h| h.interval.as_deref()),
			hc.and_then(|h| h.start_period.as_deref()),
			hc.and_then(|h| h.retries),
		);
		// `--wait-timeout` extends the poll budget so it, not the (often shorter)
		// healthcheck interval×retries plan, decides when the wait gives up.
		let iterations = effective_iterations(poll_secs, plan_iters, wait_timeout);

		// One inspect decides the short-circuits: already healthy, or no effective
		// healthcheck at all (image or compose) — in which case a server-side
		// `wait?condition=healthy` would block forever, so treat it as satisfied.
		let info = self
			.client
			.get_json::<crate::libpod::types::container::ContainerInspect>(&format!(
				"{API_PREFIX}/containers/{}/json",
				crate::libpod::urlencoded(container_name),
			))
			.await
			.map_err(ComposeError::Podman)?;
		match classify_health(&info) {
			HealthVerdict::Healthy => return Ok(()),
			HealthVerdict::NoHealthcheck => {
				tracing::debug!(
					"{container_name} has no effective healthcheck; treating service_healthy as satisfied"
				);
				return Ok(());
			}
			HealthVerdict::Failed(code) => {
				return Err(ComposeError::WaitServiceExited {
					container: container_name.to_string(),
					code,
				});
			}
			HealthVerdict::Pending => {}
		}

		// Actively drive the healthcheck on demand. A server-side
		// `wait?condition=healthy` only returns once the health *status* flips to
		// `healthy`, but Podman updates that status only when the healthcheck runs
		// — and it schedules those runs via systemd transient timers. Without
		// systemd (containers, minimal hosts) the timer never fires and the status
		// stays `starting`, so the wait would block until the whole budget elapsed.
		//
		// `GET {API_PREFIX}/containers/{name}/healthcheck` *runs* the check and
		// returns the resulting status, so polling it works with or without
		// systemd, on Podman 4.9.3 and 5.x alike. Extra on-demand runs on a
		// systemd host are harmless.
		let path = format!(
			"{API_PREFIX}/containers/{}/healthcheck",
			crate::libpod::urlencoded(container_name),
		);
		for _ in 0..iterations {
			match self.client.get_json::<HealthCheckRun>(&path).await {
				Ok(run) if run.status.as_deref() == Some("healthy") => return Ok(()),
				Ok(_) => {}
				// A transient error (container not yet running, 409, 500, …) just
				// means "not healthy yet" — keep polling rather than failing hard.
				Err(e) => tracing::debug!("{container_name} healthcheck run failed: {e}"),
			}
			tokio::time::sleep(Duration::from_secs(poll_secs)).await;
		}
		Err(ComposeError::HealthCheckTimeout(container_name.into()))
	}

	/// Wait until a container exits, then require status 0.
	///
	/// Blocks server-side on `wait?condition=stopped` (which returns the exit
	/// code) instead of polling inspect, bounded by a 600 s client-side timeout.
	/// Errors if the container exits non-zero or the deadline is exceeded.
	pub(super) async fn wait_completed(&self, container_name: &str) -> Result<()> {
		let path = format!(
			"{API_PREFIX}/containers/{}/wait?condition=stopped",
			crate::libpod::urlencoded(container_name),
		);
		let budget = std::time::Duration::from_secs(600);
		match tokio::time::timeout(budget, self.client.post_empty_json_unbounded::<i64>(&path))
			.await
		{
			Ok(Ok(0)) => Ok(()),
			Ok(Ok(code)) => Err(ComposeError::HealthCheckTimeout(format!(
				"{container_name} exited with non-zero status {code}"
			))),
			Ok(Err(e)) => {
				tracing::debug!("{container_name} wait?condition=stopped failed: {e}");
				Err(ComposeError::HealthCheckTimeout(container_name.into()))
			}
			Err(_elapsed) => Err(ComposeError::HealthCheckTimeout(container_name.into())),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn inspect(json: &str) -> ContainerInspect {
		serde_json::from_str(json).expect("fixture parses")
	}

	// --- wait_healthy poll plan (#418) ---------------------------------------

	#[test]
	fn poll_plan_defaults_match_legacy_60s() {
		// No healthcheck timing set → 2s poll, 30 probes (the historical budget).
		assert_eq!(super::health_poll_plan(None, None, None), (2, 30));
	}

	#[test]
	fn poll_plan_uses_interval_and_honors_start_period() {
		// interval=10s, start_period=60s, retries=3 → poll 10s, 3 + 60/10 = 9 probes.
		let (poll, iters) = super::health_poll_plan(Some("10s"), Some("60s"), Some(3));
		assert_eq!((poll, iters), (10, 9));
	}

	#[test]
	fn poll_plan_sub_second_interval_floors_to_default() {
		// An interval below 1s falls back to the 2s default (no busy-poll).
		let (poll, _) = super::health_poll_plan(Some("500ms"), None, Some(5));
		assert_eq!(poll, 2);
	}

	// --- effective_iterations (--wait-timeout budget, #891) ------------------

	#[test]
	fn effective_iterations_without_wait_timeout_uses_plan() {
		// No --wait-timeout: the healthcheck poll plan governs unchanged.
		assert_eq!(super::effective_iterations(2, 30, None), 30);
	}

	#[test]
	fn effective_iterations_extends_to_cover_wait_timeout() {
		// A short plan (interval 10s × 1 retry = 1 iter) must be extended so a
		// generous --wait-timeout actually elapses: 120s / 10s + 2 margin = 14.
		let iters = super::effective_iterations(10, 1, Some(Duration::from_secs(120)));
		assert_eq!(iters, 14);
	}

	#[test]
	fn effective_iterations_keeps_larger_plan() {
		// When the poll plan already outlasts --wait-timeout, the plan wins so the
		// healthcheck's own budget is never shortened.
		let iters = super::effective_iterations(2, 100, Some(Duration::from_secs(10)));
		assert_eq!(iters, 100);
	}

	// --- wait_healthy gating (service_healthy) -------------------------------

	#[test]
	fn health_reported_healthy() {
		let info = inspect(r#"{"State":{"Status":"running","Health":{"Status":"healthy"}}}"#);
		assert!(matches!(classify_health(&info), HealthVerdict::Healthy));
	}

	#[test]
	fn health_no_effective_healthcheck_is_satisfied() {
		// A disabled healthcheck (Test ["NONE"]) can never report healthy, so the
		// dependency short-circuits as satisfied rather than blocking to timeout.
		let info =
			inspect(r#"{"State":{"Status":"running"},"Config":{"Healthcheck":{"Test":["NONE"]}}}"#);
		assert!(matches!(
			classify_health(&info),
			HealthVerdict::NoHealthcheck
		));
	}

	#[test]
	fn health_starting_with_healthcheck_pends() {
		let info = inspect(
			r#"{"State":{"Status":"running","Health":{"Status":"starting"}},"Config":{"Healthcheck":{"Test":["CMD","true"]}}}"#,
		);
		assert!(matches!(classify_health(&info), HealthVerdict::Pending));
	}

	#[test]
	fn health_exited_nonzero_fails() {
		// A no-healthcheck service that crashed during the wait must fail, not be
		// reported satisfied (the `up --wait` masking bug).
		let info = inspect(r#"{"State":{"Status":"exited","ExitCode":7}}"#);
		assert!(matches!(classify_health(&info), HealthVerdict::Failed(7)));
	}

	#[test]
	fn health_exited_zero_is_satisfied() {
		// A one-shot that completed cleanly with no healthcheck is still satisfied.
		let info = inspect(r#"{"State":{"Status":"exited","ExitCode":0}}"#);
		assert!(matches!(
			classify_health(&info),
			HealthVerdict::NoHealthcheck
		));
	}
}
