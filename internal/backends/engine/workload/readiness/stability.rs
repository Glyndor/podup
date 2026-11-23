//! What `up --wait` asks of a service that has no healthcheck: that it is
//! running, and that it stays running for a short window.
//!
//! A healthcheck says when a service is ready. Without one, "ready" can only
//! mean "started and not dead", and a container in a crash loop is started a
//! moment before each of its deaths: `sh -c 'sleep 1; exit 3'` under
//! `restart: unless-stopped` is `running` when the wait looks at it and is
//! restarted a second later, so a wait that reads the state once reports a
//! success the service never had. (`docker compose up --wait` reads it once
//! too: measured against the same Podman 6.2.0 socket it returns 0 for that
//! service in 0.7 s, and fails with `exited (N)` when the container has already
//! died when it looks. podup looks again.)
//!
//! A service whose restart policy never starts it again has nothing to watch
//! for after the first look: it is running, or it ended, and a one-shot that
//! ended with status 0 under `restart: "no"` is a success.

use std::time::Duration;

use crate::error::{ComposeError, Result};
use crate::libpod::types::container::ContainerInspect;
use crate::libpod::API_PREFIX;

use crate::engine::Engine;

/// How long a service with a restart policy is watched for another start.
///
/// Podman starts a container again within a fraction of a second of its death
/// (measured on 6.2.0: a container that exits at once is restarted five times a
/// second), so a crash loop of any period under this shows a new
/// `RestartCount` inside the window. A loop slower than that, or a crash later
/// than that, is not seen by any window; the number is the price of a wait that
/// does not stall every `restart: unless-stopped` service for long.
pub(super) const STABILITY_WINDOW: Duration = Duration::from_secs(3);

/// How often the state is read inside the window.
const POLL: Duration = Duration::from_millis(150);

/// What one read of a container says to a wait for a service with no healthcheck.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Reading {
	/// Running, and its restart policy can start it again: watch it for the window.
	Running { restarts: i64, can_restart: bool },
	/// Ended with a status other than 0.
	Exited(i64),
	/// Between two runs of a restart policy, or ended with 0 under a policy that
	/// starts it again: it will not stay up.
	Restarting { code: Option<i64>, restarts: i64 },
	/// A one-shot that finished with status 0, or a state that is neither running
	/// nor ended (created, paused): nothing this wait has an opinion about.
	Fine,
}

/// Classify one inspect. Pure, so the rules are tested without a socket.
pub(super) fn read(info: &ContainerInspect) -> Reading {
	let state = info.state.as_ref();
	let status = state.and_then(|s| s.status.as_deref()).unwrap_or("");
	let code = state.and_then(|s| s.exit_code);
	let restarts = info.restart_count.unwrap_or(0);
	let policy = info
		.host_config
		.as_ref()
		.and_then(|h| h.restart_policy.as_ref())
		.and_then(|p| p.name.as_deref())
		.unwrap_or("");
	// `always` and `unless-stopped` start the container again whatever it
	// exited with; `on-failure` only after a failure, so a clean exit under it
	// is the end, as it is under `no`.
	let always_restarts = matches!(policy, "always" | "unless-stopped");
	let can_restart = always_restarts || policy == "on-failure";
	match status {
		"running" => Reading::Running {
			restarts,
			can_restart,
		},
		"restarting" => Reading::Restarting { code, restarts },
		// Podman reports `stopped`, not `exited`, for a container its restart
		// policy is about to start again (measured on 6.2.0).
		"exited" | "stopped" | "dead" => match code.unwrap_or(0) {
			0 if always_restarts => Reading::Restarting { code, restarts },
			0 => Reading::Fine,
			n if status == "stopped" && can_restart && restarts > 0 => Reading::Restarting {
				code: Some(n),
				restarts,
			},
			n => Reading::Exited(n),
		},
		_ => Reading::Fine,
	}
}

impl Engine {
	/// Hold a started container with no healthcheck to what `--wait` promises:
	/// running now, and, when a restart policy could start it again, still on
	/// its first run [`STABILITY_WINDOW`] later (bounded by `limit`, the time
	/// the caller has left).
	///
	/// A container with a healthcheck is not asked: its health is the answer.
	pub(in crate::backends::engine) async fn wait_stable(
		&self,
		container: &str,
		limit: Option<Duration>,
	) -> Result<()> {
		let path = format!(
			"{API_PREFIX}/containers/{}/json",
			crate::libpod::urlencoded(container)
		);
		let first: ContainerInspect = self
			.client
			.get_json(&path)
			.await
			.map_err(ComposeError::Podman)?;
		if first.config.as_ref().is_some_and(|c| c.has_healthcheck()) {
			return Ok(());
		}
		let (baseline, can_restart) = match read(&first) {
			Reading::Running {
				restarts,
				can_restart,
			} => (restarts, can_restart),
			other => return verdict(container, other),
		};
		if !can_restart {
			return Ok(());
		}
		let window = limit.map_or(STABILITY_WINDOW, |l| l.min(STABILITY_WINDOW));
		let deadline = tokio::time::Instant::now() + window;
		while tokio::time::Instant::now() < deadline {
			tokio::time::sleep(
				POLL.min(deadline.saturating_duration_since(tokio::time::Instant::now())),
			)
			.await;
			let Ok(info) = self.client.get_json::<ContainerInspect>(&path).await else {
				continue;
			};
			match read(&info) {
				Reading::Running { restarts, .. } if restarts > baseline => {
					return Err(ComposeError::WaitServiceRestarting {
						container: container.to_string(),
						code: None,
						restarts,
					})
				}
				Reading::Running { .. } => {}
				other => return verdict(container, other),
			}
		}
		Ok(())
	}
}

/// The error a reading other than "running" stands for, or `Ok` for one that is fine.
fn verdict(container: &str, reading: Reading) -> Result<()> {
	match reading {
		Reading::Exited(code) => Err(ComposeError::WaitServiceExited {
			container: container.to_string(),
			code,
		}),
		Reading::Restarting { code, restarts } => Err(ComposeError::WaitServiceRestarting {
			container: container.to_string(),
			code,
			restarts,
		}),
		Reading::Running { .. } | Reading::Fine => Ok(()),
	}
}

#[cfg(test)]
#[path = "stability_tests.rs"]
mod tests;
