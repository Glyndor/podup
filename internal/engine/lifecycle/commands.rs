//! Lifecycle sub-commands: restart, stop, start, kill, rm, pause, unpause, run.

use std::io::Write;

use futures_util::StreamExt;
use tracing::info;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};

use super::{filter_services, RunOptions};
use crate::engine::Engine;
use crate::libpod::API_PREFIX;

impl Engine {
	/// Run a lifecycle POST against one container with a consistent outcome:
	/// success logs `{done} {container}`; "already in the desired state" (304)
	/// and "no such container" (404) are idempotent no-ops; any other failure is
	/// a real error that propagates (setting a non-zero exit) instead of being
	/// swallowed into a warning. Shared by stop/start/restart/kill/pause/unpause
	/// so they all behave the same.
	async fn run_lifecycle_op(&self, path: &str, container: &str, done: &str) -> Result<()> {
		match self.client.post_empty_ok(path).await {
			Ok(()) => {
				info!("{done} {container}");
				Ok(())
			}
			Err(e) if e.is_status(304) || e.is_status(404) => {
				tracing::debug!("{container}: {done} skipped ({e})");
				Ok(())
			}
			Err(e) => Err(ComposeError::Podman(e)),
		}
	}

	/// Restart the named service (or all services). Dependents with a `restart` condition in `depends_on` are also restarted.
	pub async fn restart(&self, file: &ComposeFile, service_name: Option<&str>) -> Result<()> {
		let targets: Vec<String> = service_name
			.map(|s| vec![s.to_string()])
			.unwrap_or_default();
		self.restart_with_options(file, &targets, false).await
	}

	/// Restart with options. When `target_services` is empty, all services are
	/// restarted. When `no_deps` is true, dependents with a `depends_on` restart
	/// condition are NOT cascade-restarted.
	pub async fn restart_with_options(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		no_deps: bool,
	) -> Result<()> {
		let order = crate::compose::resolve_order(file)?;
		let names = filter_services(file, order, target_services)?;

		for name in &names {
			let service = &file.services[name];

			for container_name in self.live_replica_names(name, service).await? {
				let grace = self.grace_period_secs(service);
				// Single atomic restart (no visible stopped window) instead of a
				// stop+start round-trip.
				let restart_path = format!(
					"{API_PREFIX}/containers/{}/restart?t={grace}",
					crate::libpod::urlencoded(&container_name),
				);
				self.run_lifecycle_op(&restart_path, &container_name, "restarted")
					.await?;
			}

			if no_deps {
				continue;
			}
			for (dep_name, dep_service) in &file.services {
				if dep_service.depends_on.restart_for(name) {
					for dep_container in self.live_replica_names(dep_name, dep_service).await? {
						let grace = self.grace_period_secs(dep_service);
						let restart_path = format!(
							"{API_PREFIX}/containers/{}/restart?t={grace}",
							crate::libpod::urlencoded(&dep_container),
						);
						if let Err(e) = self.client.post_empty_ok(&restart_path).await {
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

	/// Block until each targeted service's containers stop, printing each exit
	/// code (`docker compose wait`). Returns `RunExited` with the last non-zero
	/// code so the process exit status reflects it, mirroring docker compose.
	pub async fn wait_services(
		&self,
		file: &ComposeFile,
		target_services: &[String],
	) -> Result<()> {
		let order = crate::compose::resolve_order(file)?;
		let order = filter_services(file, order, target_services)?;

		let mut last_nonzero = 0i64;
		for name in &order {
			let service = &file.services[name];
			for container_name in self.live_replica_names(name, service).await? {
				let path = format!(
					"{API_PREFIX}/containers/{}/wait?condition=stopped",
					crate::libpod::urlencoded(&container_name),
				);
				let code = self
					.client
					.post_empty_json_unbounded::<i64>(&path)
					.await
					.map_err(ComposeError::Podman)?;
				println!("{code}");
				if code != 0 {
					last_nonzero = code;
				}
			}
		}
		if last_nonzero != 0 {
			return Err(ComposeError::RunExited(last_nonzero));
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
			for container_name in self.live_replica_names(name, service).await? {
				let grace = self.grace_period_secs(service);
				let path = format!(
					"{API_PREFIX}/containers/{}/stop?t={grace}",
					crate::libpod::urlencoded(&container_name),
				);
				self.run_lifecycle_op(&path, &container_name, "stopped")
					.await?;
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
			for container_name in self.live_replica_names(name, service).await? {
				let path = format!(
					"{API_PREFIX}/containers/{}/start",
					crate::libpod::urlencoded(&container_name),
				);
				self.run_lifecycle_op(&path, &container_name, "started")
					.await?;
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
			for container_name in self.live_replica_names(name, service).await? {
				let path = format!(
					"{API_PREFIX}/containers/{}/kill?signal={}",
					crate::libpod::urlencoded(&container_name),
					crate::libpod::urlencoded(signal),
				);
				self.run_lifecycle_op(&path, &container_name, &format!("sent {signal} to"))
					.await?;
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
		self.rm_with_options(file, target_services, force, false)
			.await
	}

	/// Remove stopped service containers. `remove_volumes` (`-v/--volumes`) also
	/// removes anonymous volumes attached to each container.
	pub async fn rm_with_options(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		force: bool,
		remove_volumes: bool,
	) -> Result<()> {
		let mut order = crate::compose::resolve_order(file)?;
		order.reverse();
		let order = filter_services(file, order, target_services)?;

		for name in &order {
			let service = &file.services[name];
			for container_name in self.live_replica_names(name, service).await? {
				let force_str = if force { "true" } else { "false" };
				let path = format!(
					"{API_PREFIX}/containers/{}?force={force_str}&v={remove_volumes}",
					crate::libpod::urlencoded(&container_name),
				);
				if let Err(e) = self.client.delete_ok(&path).await {
					tracing::warn!("could not remove {container_name}: {e}");
				} else {
					info!("removed {container_name}");
				}
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
			for container_name in self.live_replica_names(name, service).await? {
				let path = format!(
					"{API_PREFIX}/containers/{}/pause",
					crate::libpod::urlencoded(&container_name),
				);
				self.run_lifecycle_op(&path, &container_name, "paused")
					.await?;
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
			for container_name in self.live_replica_names(name, service).await? {
				let path = format!(
					"{API_PREFIX}/containers/{}/unpause",
					crate::libpod::urlencoded(&container_name),
				);
				self.run_lifecycle_op(&path, &container_name, "unpaused")
					.await?;
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
			service_ports,
		} = opts;
		// CLI-only run flags arrive via the engine builder (see `RunOverrides`),
		// keeping the public `RunOptions` API frozen at 1.0.
		let super::RunOverrides {
			user,
			workdir,
			entrypoint,
			volumes,
			publish,
			interactive,
			no_deps,
		} = self.run_overrides.clone();
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;

		// Compose `run` brings up the service's `depends_on` services first (and
		// waits on their conditions), unless `--no-deps` is given. The service
		// itself is excluded — only its transitive dependencies are started.
		if !no_deps {
			let deps: Vec<String> = super::expand_targets(file, &[service_name.to_string()], false)
				.map(|set| set.into_iter().filter(|n| n != service_name).collect())
				.unwrap_or_default();
			if !deps.is_empty() {
				self.up_with_options(file, true, &[], &deps, false, false, false)
					.await?;
			}
		}

		let run_name = name_override.unwrap_or_else(|| {
			format!("{}-{service_name}-run-{}", self.project, std::process::id())
		});

		let mut run_service = service.clone();
		if !cmd.is_empty() {
			run_service.command = Some(crate::compose::types::Command::Exec(cmd));
		}
		// `--entrypoint` overrides the image/service entrypoint with a single
		// executable token (compose/`docker run` semantics); any `cmd` becomes
		// its arguments.
		if let Some(ep) = entrypoint {
			run_service.entrypoint = Some(crate::compose::types::Command::Exec(vec![ep]));
		}
		if let Some(u) = user {
			run_service.user = Some(u);
		}
		if let Some(w) = workdir {
			run_service.working_dir = Some(w);
		}
		// `-i/--interactive` keeps STDIN open on the spec; `run` still streams
		// logs rather than attaching a live terminal.
		if interactive {
			run_service.stdin_open = Some(true);
		}
		// Ad-hoc `-v/--volume` mounts append to the service's own mounts in
		// compose short form, parsed downstream like compose file entries.
		for v in volumes {
			run_service
				.volumes
				.push(crate::compose::types::VolumeMount::Short(v));
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
		// Compose `run` does not publish the service's ports unless
		// `--service-ports` is given; otherwise a one-off run would collide
		// with the long-running service's host-port bindings.
		if !service_ports {
			run_service.ports.clear();
		}
		// Explicit `-p/--publish` ports are always bound, even without
		// `--service-ports`, matching `docker compose run -p`.
		for p in publish {
			run_service
				.ports
				.push(crate::compose::types::PortMapping::Short(p));
		}
		// Force non-TTY so Podman uses multiplexed log framing that
		// parse_multiplexed can decode. TTY mode sends raw bytes without
		// the 8-byte header, which would produce garbled output.
		run_service.tty = None;

		// Ensure the project networks exist (compose `run` brings them up like
		// `up` does); the service may reference the synthesized `default`
		// network, which is created here as `{project}_default`.
		self.create_networks(file).await?;
		// Inline secrets/configs are created up front (no longer in the
		// per-container build path), so materialise them here too before the run
		// container is created.
		self.create_inline_secrets(file).await?;

		self.create_and_start(&run_name, service_name, &run_service, file, true)
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

		// Lock stdout once for the whole stream instead of re-acquiring the lock
		// (and issuing a syscall) per frame; stdout is ours exclusively on this
		// path. stderr is locked per frame because the tracing subscriber also
		// writes there: holding its lock across the await loop would starve
		// concurrent log emissions. Flush after each frame so `run` streams
		// promptly.
		let mut out = std::io::stdout().lock();
		while let Some(msg) = log_stream.next().await {
			match msg.map_err(ComposeError::Podman)? {
				crate::libpod::LogOutput::StdOut { message } => {
					let _ = out.write_all(String::from_utf8_lossy(&message).as_bytes());
					let _ = out.flush();
				}
				crate::libpod::LogOutput::StdErr { message } => {
					let mut err = std::io::stderr().lock();
					let _ = err.write_all(String::from_utf8_lossy(&message).as_bytes());
					let _ = err.flush();
				}
			}
		}

		let wait_path = format!(
			"{API_PREFIX}/containers/{}/wait?condition=stopped",
			crate::libpod::urlencoded(&run_name),
		);
		// Capture the wait result before cleanup so a failed wait is surfaced as an
		// error rather than masked as a successful (exit 0) run.
		let wait_result = self
			.client
			.post_empty_json_unbounded::<i64>(&wait_path)
			.await;

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
