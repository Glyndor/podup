//! Health and completion polling for service dependency ordering.
//!
//! [`Engine::wait_healthy`] polls until the container reports `healthy` (used when
//! a dependent service declares `condition: service_healthy`).
//! [`Engine::wait_completed`] polls until the container exits with code 0 (used for
//! `condition: service_completed_successfully`).

use crate::compose::types::{ComposeFile, Service};
use crate::error::{ComposeError, Result};
use crate::libpod::types::container::{ContainerInspect, ContainerState};
use crate::libpod::API_PREFIX;

use super::Engine;

/// Per-poll verdict while waiting for `service_healthy`.
enum HealthVerdict {
	/// The runtime reports the container as `healthy`.
	Healthy,
	/// The container has no effective healthcheck, so `healthy` is unreachable;
	/// treat the dependency as satisfied rather than blocking until timeout.
	NoHealthcheck,
	/// Not healthy yet — keep polling.
	Pending,
}

/// Per-poll verdict while waiting for `service_completed_successfully`.
enum CompletionVerdict {
	/// The container exited with status 0.
	Succeeded,
	/// The container exited with a non-zero status — the dependency failed.
	Failed,
	/// Not exited yet — keep polling.
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

/// Classify a container state while waiting for `service_completed_successfully`.
///
/// Pure decision logic for [`Engine::wait_completed`], split out so the
/// fail-closed-on-non-zero-exit gating can be unit-tested without Podman. A
/// missing exit code is treated as failure (`unwrap_or(-1)`).
fn classify_completion(state: Option<&ContainerState>) -> CompletionVerdict {
	match state {
		Some(s) if s.status.as_deref() == Some("exited") => {
			if s.exit_code.unwrap_or(-1) == 0 {
				CompletionVerdict::Succeeded
			} else {
				CompletionVerdict::Failed
			}
		}
		_ => CompletionVerdict::Pending,
	}
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

impl Engine {
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
	/// Wait until every targeted service's first replica is healthy (`up
	/// --wait`). A service with no effective healthcheck is treated as ready
	/// once started. All services when `target_services` is empty.
	pub async fn wait_services_healthy(
		&self,
		file: &ComposeFile,
		target_services: &[String],
	) -> Result<()> {
		for (name, service) in &file.services {
			if !target_services.is_empty() && !target_services.iter().any(|t| t == name) {
				continue;
			}
			let container = self.first_replica_name(name, service);
			self.wait_healthy(&container, service).await?;
		}
		Ok(())
	}

	pub(super) async fn wait_healthy(&self, container_name: &str, service: &Service) -> Result<()> {
		let hc = service.healthcheck.as_ref();
		let (poll_secs, iterations) = health_poll_plan(
			hc.and_then(|h| h.interval.as_deref()),
			hc.and_then(|h| h.start_period.as_deref()),
			hc.and_then(|h| h.retries),
		);

		for _ in 0..iterations {
			let info = match self
				.client
				.get_json::<crate::libpod::types::container::ContainerInspect>(&format!(
					"{API_PREFIX}/containers/{}/json",
					crate::libpod::urlencoded(container_name),
				))
				.await
			{
				Ok(i) => i,
				Err(e) => {
					tracing::debug!("inspect error (will retry): {e}");
					tokio::time::sleep(std::time::Duration::from_secs(poll_secs)).await;
					continue;
				}
			};
			match classify_health(&info) {
				HealthVerdict::Healthy => return Ok(()),
				HealthVerdict::NoHealthcheck => {
					tracing::debug!(
						"{container_name} has no effective healthcheck; treating service_healthy as satisfied"
					);
					return Ok(());
				}
				HealthVerdict::Pending => {}
			}
			tokio::time::sleep(std::time::Duration::from_secs(poll_secs)).await;
		}

		Err(ComposeError::HealthCheckTimeout(container_name.into()))
	}

	/// Poll a container until it exits with status 0.
	///
	/// Tries for up to 600 seconds (1 s interval). Errors if the container
	/// exits with a non-zero code or if the deadline is exceeded.
	pub(super) async fn wait_completed(&self, container_name: &str) -> Result<()> {
		for _ in 0..600 {
			let info = match self
				.client
				.get_json::<crate::libpod::types::container::ContainerInspect>(&format!(
					"{API_PREFIX}/containers/{}/json",
					crate::libpod::urlencoded(container_name),
				))
				.await
			{
				Ok(i) => i,
				Err(e) => {
					tracing::debug!("inspect error (will retry): {e}");
					tokio::time::sleep(std::time::Duration::from_secs(1)).await;
					continue;
				}
			};
			match classify_completion(info.state.as_ref()) {
				CompletionVerdict::Succeeded => return Ok(()),
				CompletionVerdict::Failed => {
					return Err(ComposeError::HealthCheckTimeout(format!(
						"{container_name} exited with non-zero status"
					)));
				}
				CompletionVerdict::Pending => {}
			}
			tokio::time::sleep(std::time::Duration::from_secs(1)).await;
		}
		Err(ComposeError::HealthCheckTimeout(container_name.into()))
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

	// --- wait_completed gating (service_completed_successfully) ---------------

	#[test]
	fn completion_exited_zero_succeeds() {
		let info = inspect(r#"{"State":{"Status":"exited","ExitCode":0}}"#);
		assert!(matches!(
			classify_completion(info.state.as_ref()),
			CompletionVerdict::Succeeded
		));
	}

	#[test]
	fn completion_exited_nonzero_fails() {
		let info = inspect(r#"{"State":{"Status":"exited","ExitCode":1}}"#);
		assert!(matches!(
			classify_completion(info.state.as_ref()),
			CompletionVerdict::Failed
		));
	}

	#[test]
	fn completion_exited_missing_code_fails_closed() {
		// No ExitCode → unwrap_or(-1) → must be treated as failure, never success.
		let info = inspect(r#"{"State":{"Status":"exited"}}"#);
		assert!(matches!(
			classify_completion(info.state.as_ref()),
			CompletionVerdict::Failed
		));
	}

	#[test]
	fn completion_running_pends() {
		let info = inspect(r#"{"State":{"Status":"running","ExitCode":0}}"#);
		assert!(matches!(
			classify_completion(info.state.as_ref()),
			CompletionVerdict::Pending
		));
	}

	#[test]
	fn completion_no_state_pends() {
		let info = inspect("{}");
		assert!(matches!(
			classify_completion(info.state.as_ref()),
			CompletionVerdict::Pending
		));
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
}
