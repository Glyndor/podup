//! Health and completion polling for service dependency ordering.
//!
//! [`Engine::wait_healthy`] polls until the container reports `healthy` (used when
//! a dependent service declares `condition: service_healthy`).
//! [`Engine::wait_completed`] polls until the container exits with code 0 (used for
//! `condition: service_completed_successfully`).

use crate::compose::types::Service;
use crate::error::{ComposeError, Result};

use super::Engine;

impl Engine {
    /// Poll a container until its health status is `healthy` or timeout.
    ///
    /// Uses `healthcheck.retries` (default 30) with a 2 s interval between probes.
    pub(super) async fn wait_healthy(&self, container_name: &str, service: &Service) -> Result<()> {
        use bollard::models::HealthStatusEnum;

        let retries = service
            .healthcheck
            .as_ref()
            .and_then(|h| h.retries)
            .unwrap_or(30);

        for _ in 0..retries {
            let info = match self.docker.inspect_container(container_name, None).await {
                Ok(i) => i,
                Err(e) => {
                    // Podman uses "stopped" for exited containers; Bollard can't
                    // deserialize it. Treat any inspect error as "not healthy yet".
                    tracing::debug!("inspect_container error (will retry): {e}");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
            };
            if let Some(state) = info.state {
                if let Some(health) = state.health {
                    if health.status == Some(HealthStatusEnum::HEALTHY) {
                        return Ok(());
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }

        Err(ComposeError::HealthCheckTimeout(container_name.into()))
    }

    /// Poll a container until it exits with status 0.
    ///
    /// Tries for up to 600 seconds (1 s interval). Errors if the container
    /// exits with a non-zero code or if the deadline is exceeded.
    pub(super) async fn wait_completed(&self, container_name: &str) -> Result<()> {
        for _ in 0..600 {
            let info = match self.docker.inspect_container(container_name, None).await {
                Ok(i) => i,
                Err(e) => {
                    tracing::debug!("inspect_container error (will retry): {e}");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            };
            if let Some(state) = info.state {
                let status = state.status.map(|s| format!("{s:?}").to_lowercase());
                if status.as_deref() == Some("exited") {
                    if state.exit_code.unwrap_or(-1) == 0 {
                        return Ok(());
                    }
                    return Err(ComposeError::HealthCheckTimeout(format!(
                        "{container_name} exited with non-zero status"
                    )));
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        Err(ComposeError::HealthCheckTimeout(container_name.into()))
    }
}
