use std::future::Future;

use super::Engine;
use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, PodmanError, API_PREFIX};

impl Engine {
	pub(super) async fn abort_exit_container(
		&self,
		file: &ComposeFile,
		service: &str,
		trigger_service: &str,
		trigger_container: &str,
	) -> Result<String> {
		let names = self.replica_names(service, &file.services[service]);
		select_exit_container(
			service,
			trigger_service,
			trigger_container,
			&names,
			|name| async move {
				let path = format!(
					"{API_PREFIX}/containers/json?all=true&filters={}",
					self.project_label_filter_encoded()
				);
				let entries = self
					.client
					.get_json::<Vec<crate::libpod::types::container::ContainerListEntry>>(&path)
					.await
					.map_err(|e| exit_read_error(service, &name, e))?;
				Ok(entries.iter().any(|entry| {
					entry
						.names
						.iter()
						.any(|raw| raw.trim_start_matches('/') == name)
						&& matches!(entry.state.as_str(), "exited" | "stopped")
				}))
			},
		)
		.await
	}

	pub(super) async fn container_exit_code(&self, service: &str, name: &str) -> Result<i64> {
		let path = format!(
			"{API_PREFIX}/containers/{}/wait?condition=stopped",
			urlencoded(name)
		);
		exit_code_result(
			service,
			name,
			self.client.post_empty_json_unbounded::<i64>(&path).await,
		)
	}
}

pub(super) fn exit_code_result(
	service: &str,
	container: &str,
	result: std::result::Result<i64, PodmanError>,
) -> Result<i64> {
	result.map_err(|e| exit_read_error(service, container, e))
}

fn exit_read_error(service: &str, container: &str, error: PodmanError) -> ComposeError {
	ComposeError::Io(std::io::Error::other(format!(
		"service {service}: could not read exit status of {container}: {error}"
	)))
}

pub(super) async fn select_exit_container<F, Fut>(
	service: &str,
	trigger_service: &str,
	trigger_container: &str,
	names: &[String],
	mut stopped: F,
) -> Result<String>
where
	F: FnMut(String) -> Fut,
	Fut: Future<Output = Result<bool>>,
{
	// Prefer the triggering replica. For another service, use its first
	// observed stopped replica (replica order breaks ties), before teardown;
	// if none has stopped, fall back to its first replica.
	if service == trigger_service {
		return Ok(trigger_container.to_string());
	}
	for name in names {
		if stopped(name.clone()).await? {
			return Ok(name.clone());
		}
	}
	names
		.first()
		.cloned()
		.ok_or_else(|| ComposeError::ServiceNotFound(service.to_string()))
}

pub(super) async fn finish_abort(
	wait: impl Future<Output = Result<i64>>,
	stop: impl Future<Output = Result<()>>,
) -> Result<i64> {
	// Cleanup must run even when the status is unknown. Poll concurrently so
	// a running fallback replica can exit as a result of this very stop.
	let (code, stopped) = futures_util::future::join(wait, stop).await;
	if let Err(error) = stopped {
		tracing::warn!("abort-on-container-exit: stop after container exited: {error}");
	}
	code
}

#[cfg(test)]
mod tests;

#[cfg(all(test, unix))]
mod engine_tests;
