//! Lifecycle sub-commands: restart, stop, start, kill, rm, pause, unpause, run.

use futures_util::StreamExt;
use tracing::info;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};

use super::{filter_services, grace_period_secs, RunOptions};
use crate::engine::Engine;
use crate::libpod::API_PREFIX;

impl Engine {
	/// Restart the named service (or all services). Dependents with a `restart` condition in `depends_on` are also restarted.
	pub async fn restart(&self, file: &ComposeFile, service_name: Option<&str>) -> Result<()> {
		let names: Vec<String> = if let Some(svc) = service_name {
			if !file.services.contains_key(svc) {
				return Err(ComposeError::ServiceNotFound(svc.into()));
			}
			vec![svc.to_string()]
		} else {
			file.services.keys().cloned().collect()
		};

		for name in &names {
			let service = &file.services[name];

			for container_name in self.replica_names(name, service) {
				let grace = grace_period_secs(service);
				let stop_path = format!(
					"{API_PREFIX}/containers/{}/stop?t={grace}",
					crate::libpod::urlencoded(&container_name),
				);
				if let Err(e) = self.client.post_empty_ok(&stop_path).await {
					tracing::debug!("stop before restart {container_name}: {e}");
				}

				let start_path = format!(
					"{API_PREFIX}/containers/{}/start",
					crate::libpod::urlencoded(&container_name),
				);
				self.client
					.post_empty_ok(&start_path)
					.await
					.map_err(ComposeError::Podman)?;

				info!("restarted {container_name}");
			}

			for (dep_name, dep_service) in &file.services {
				if dep_service.depends_on.restart_for(name) {
					for dep_container in self.replica_names(dep_name, dep_service) {
						let grace = grace_period_secs(dep_service);
						let stop_path = format!(
							"{API_PREFIX}/containers/{}/stop?t={grace}",
							crate::libpod::urlencoded(&dep_container),
						);
						if let Err(e) = self.client.post_empty_ok(&stop_path).await {
							tracing::debug!("stop before cascade restart {dep_container}: {e}");
						}
						let start_path = format!(
							"{API_PREFIX}/containers/{}/start",
							crate::libpod::urlencoded(&dep_container),
						);
						if let Err(e) = self.client.post_empty_ok(&start_path).await {
							tracing::warn!("cascade restart of {dep_name} failed: {e}");
						} else {
							info!("cascade-restarted {dep_container} (depends_on.restart)");
						}
					}
				}
			}
		}

		Ok(())
	}

	/// Stop running containers without removing them.
	///
	/// Services are stopped in reverse dependency order. If `target_services`
	/// is empty, all services in the compose file are stopped.
	pub async fn stop(&self, file: &ComposeFile, target_services: &[String]) -> Result<()> {
		let mut order = crate::compose::resolve_order(file)?;
		order.reverse();
		let order = filter_services(file, order, target_services)?;

		for name in &order {
			let service = &file.services[name];
			for container_name in self.replica_names(name, service) {
				let grace = grace_period_secs(service);
				let path = format!(
					"{API_PREFIX}/containers/{}/stop?t={grace}",
					crate::libpod::urlencoded(&container_name),
				);
				if let Err(e) = self.client.post_empty_ok(&path).await {
					tracing::debug!("stop {container_name}: {e}");
				}
				info!("stopped {container_name}");
			}
		}
		Ok(())
	}

	/// Start stopped containers.
	///
	/// Services are started in dependency order. If `target_services` is empty,
	/// all services in the compose file are started.
	pub async fn start(&self, file: &ComposeFile, target_services: &[String]) -> Result<()> {
		let order = crate::compose::resolve_order(file)?;
		let order = filter_services(file, order, target_services)?;

		for name in &order {
			let service = &file.services[name];
			for container_name in self.replica_names(name, service) {
				let path = format!(
					"{API_PREFIX}/containers/{}/start",
					crate::libpod::urlencoded(&container_name),
				);
				self.client
					.post_empty_ok(&path)
					.await
					.map_err(ComposeError::Podman)?;
				info!("started {container_name}");
			}
		}
		Ok(())
	}

	/// Send a signal to service containers (default: `SIGKILL`).
	///
	/// If `target_services` is empty, all services are signalled.
	pub async fn kill(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		signal: &str,
	) -> Result<()> {
		let order = crate::compose::resolve_order(file)?;
		let order = filter_services(file, order, target_services)?;

		for name in &order {
			let service = &file.services[name];
			for container_name in self.replica_names(name, service) {
				let path = format!(
					"{API_PREFIX}/containers/{}/kill?signal={}",
					crate::libpod::urlencoded(&container_name),
					crate::libpod::urlencoded(signal),
				);
				self.client
					.post_empty_ok(&path)
					.await
					.map_err(ComposeError::Podman)?;
				info!("sent {signal} to {container_name}");
			}
		}
		Ok(())
	}

	/// Remove stopped service containers.
	///
	/// When `force` is true, running containers are stopped before removal.
	/// Services are removed in reverse dependency order.
	pub async fn rm(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		force: bool,
	) -> Result<()> {
		let mut order = crate::compose::resolve_order(file)?;
		order.reverse();
		let order = filter_services(file, order, target_services)?;

		for name in &order {
			let service = &file.services[name];
			for container_name in self.replica_names(name, service) {
				let force_str = if force { "true" } else { "false" };
				let path = format!(
					"{API_PREFIX}/containers/{}?force={force_str}",
					crate::libpod::urlencoded(&container_name),
				);
				if let Err(e) = self.client.delete_ok(&path).await {
					tracing::debug!("rm {container_name}: {e}");
				}
				info!("removed {container_name}");
			}
		}
		Ok(())
	}

	/// Pause running service containers (SIGSTOP).
	///
	/// If `target_services` is empty, all services are paused.
	pub async fn pause(&self, file: &ComposeFile, target_services: &[String]) -> Result<()> {
		let order = crate::compose::resolve_order(file)?;
		let order = filter_services(file, order, target_services)?;

		for name in &order {
			let service = &file.services[name];
			for container_name in self.replica_names(name, service) {
				let path = format!(
					"{API_PREFIX}/containers/{}/pause",
					crate::libpod::urlencoded(&container_name),
				);
				self.client
					.post_empty_ok(&path)
					.await
					.map_err(ComposeError::Podman)?;
				info!("paused {container_name}");
			}
		}
		Ok(())
	}

	/// Resume paused service containers.
	///
	/// If `target_services` is empty, all services are unpaused.
	pub async fn unpause(&self, file: &ComposeFile, target_services: &[String]) -> Result<()> {
		let order = crate::compose::resolve_order(file)?;
		let order = filter_services(file, order, target_services)?;

		for name in &order {
			let service = &file.services[name];
			for container_name in self.replica_names(name, service) {
				let path = format!(
					"{API_PREFIX}/containers/{}/unpause",
					crate::libpod::urlencoded(&container_name),
				);
				self.client
					.post_empty_ok(&path)
					.await
					.map_err(ComposeError::Podman)?;
				info!("unpaused {container_name}");
			}
		}
		Ok(())
	}

	/// Run a one-off command in a new container for a service.
	///
	/// The container is started, its output streamed, and it is removed when done
	/// (unless `opts.rm` is false). Non-zero exit codes surface as `ComposeError::RunExited`.
	pub async fn run(
		&self,
		file: &ComposeFile,
		service_name: &str,
		opts: RunOptions,
	) -> Result<()> {
		let RunOptions {
			cmd,
			rm,
			detach,
			env_overrides,
			name_override,
		} = opts;
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;

		let run_name = name_override.unwrap_or_else(|| {
			format!("{}-{service_name}-run-{}", self.project, std::process::id())
		});

		let mut run_service = service.clone();
		if !cmd.is_empty() {
			run_service.command = Some(crate::compose::types::Command::Exec(cmd));
		}
		if !env_overrides.is_empty() {
			let mut env_list: Vec<String> = {
				let map = run_service.environment.to_map();
				map.into_iter()
					.map(|(k, v)| v.map_or(k.clone(), |v| format!("{k}={v}")))
					.collect()
			};
			env_list.extend(env_overrides);
			run_service.environment = crate::compose::types::EnvVars::List(env_list);
		}
		run_service.restart = None;
		// Force non-TTY so Podman uses multiplexed log framing that
		// parse_multiplexed can decode. TTY mode sends raw bytes without
		// the 8-byte header, which would produce garbled output.
		run_service.tty = None;

		self.create_and_start(&run_name, service_name, &run_service, file)
			.await?;

		if detach {
			info!("started run container {run_name}");
			return Ok(());
		}

		let logs_path = format!(
			"{API_PREFIX}/containers/{}/logs?follow=true&stdout=true&stderr=true",
			crate::libpod::urlencoded(&run_name),
		);
		let logs_resp = self
			.client
			.get_stream(&logs_path)
			.await
			.map_err(ComposeError::Podman)?;
		let mut log_stream = crate::libpod::parse_multiplexed(logs_resp.into_body());

		while let Some(msg) = log_stream.next().await {
			match msg.map_err(ComposeError::Podman)? {
				crate::libpod::LogOutput::StdOut { message } => {
					print!("{}", String::from_utf8_lossy(&message))
				}
				crate::libpod::LogOutput::StdErr { message } => {
					eprint!("{}", String::from_utf8_lossy(&message))
				}
			}
		}

		let wait_path = format!(
			"{API_PREFIX}/containers/{}/wait?condition=stopped",
			crate::libpod::urlencoded(&run_name),
		);
		// Capture the wait result before cleanup so a failed wait is surfaced as an
		// error rather than masked as a successful (exit 0) run.
		let wait_result = self.client.post_empty_json::<i64>(&wait_path).await;

		if rm {
			let rm_path = format!(
				"{API_PREFIX}/containers/{}?force=true",
				crate::libpod::urlencoded(&run_name),
			);
			if let Err(e) = self.client.delete_ok(&rm_path).await {
				tracing::debug!("run cleanup delete {run_name}: {e}");
			}
		}

		let exit_code = wait_result.map_err(ComposeError::Podman)?;
		if exit_code != 0 {
			return Err(crate::error::ComposeError::RunExited(exit_code));
		}

		Ok(())
	}
}
