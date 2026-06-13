//! Query and observation commands: ps, logs, exec, pull, remove_orphans.

use futures_util::StreamExt;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::types::exec::{
	ExecCreateConfig, ExecCreateResponse, ExecInspect, ExecStartConfig,
};
use crate::libpod::{urlencoded, LogOutput, API_PREFIX};

use super::Engine;

mod inspect;

impl Engine {
	/// List containers for this project: name, image, command, state, and port bindings.
	pub async fn ps(&self, _file: &ComposeFile) -> Result<()> {
		let label = format!("podup.project={}", self.project);
		let filters = serde_json::json!({ "label": [label] });
		let path = format!(
			"{API_PREFIX}/containers/json?all=true&filters={}",
			urlencoded(&filters.to_string()),
		);

		let containers = self
			.client
			.get_json::<Vec<crate::libpod::types::container::ContainerListEntry>>(&path)
			.await
			.map_err(ComposeError::Podman)?;

		println!("{:<40} {:<30} {:<20}", "NAME", "IMAGE", "STATUS");
		for c in containers {
			let names = c.names.join(", ").trim_start_matches('/').to_string();
			let ports = c
				.ports
				.iter()
				.map(|p| {
					format!(
						"{}:{}->{}",
						p.host_ip.as_deref().unwrap_or(""),
						p.host_port.unwrap_or(0),
						p.container_port,
					)
				})
				.collect::<Vec<_>>()
				.join(", ");
			println!("{names:<40} {:<30} {:<20} {ports}", c.image, c.status);
		}

		Ok(())
	}

	/// Stream logs. When `service_name` is `None`, streams from all services. When `follow` is true, tails indefinitely.
	pub async fn logs(
		&self,
		file: &ComposeFile,
		service_name: Option<&str>,
		follow: bool,
	) -> Result<()> {
		// (container_name, is_tty) — TTY containers send raw bytes; non-TTY use
		// multiplexed 8-byte-header framing.
		let targets: Vec<(String, bool)> = if let Some(svc) = service_name {
			let service = file
				.services
				.get(svc)
				.ok_or_else(|| ComposeError::ServiceNotFound(svc.into()))?;
			let is_tty = service.tty.unwrap_or(false);
			self.replica_names(svc, service)
				.into_iter()
				.map(|n| (n, is_tty))
				.collect()
		} else {
			file.services
				.iter()
				.flat_map(|(n, s)| {
					let is_tty = s.tty.unwrap_or(false);
					self.replica_names(n, s)
						.into_iter()
						.map(move |cname| (cname, is_tty))
				})
				.collect()
		};

		// When follow=true, streams never end until containers stop. Run them
		// concurrently so multiple containers don't block each other.
		if follow && targets.len() > 1 {
			let futs: Vec<_> = targets
				.into_iter()
				.map(|(container_name, is_tty)| {
					let client = &self.client;
					async move {
						let path = format!(
							"{API_PREFIX}/containers/{}/logs?stdout=true&stderr=true&follow=true",
							urlencoded(&container_name),
						);
						let resp = match client.get_stream(&path).await {
							Ok(r) => r,
							Err(e) => {
								tracing::warn!("logs {container_name}: {e}");
								return;
							}
						};
						let mut stream = if is_tty {
							crate::libpod::parse_raw(resp.into_body())
						} else {
							crate::libpod::parse_multiplexed(resp.into_body())
						};
						while let Some(msg) = stream.next().await {
							match msg {
								Ok(LogOutput::StdOut { message }) => {
									print!("{}", String::from_utf8_lossy(&message));
								}
								Ok(LogOutput::StdErr { message }) => {
									eprint!("{}", String::from_utf8_lossy(&message));
								}
								Err(_) => break,
							}
						}
					}
				})
				.collect();
			futures_util::future::join_all(futs).await;
		} else {
			for (container_name, is_tty) in targets {
				let path = format!(
					"{API_PREFIX}/containers/{}/logs?stdout=true&stderr=true&follow={}",
					urlencoded(&container_name),
					follow,
				);
				let resp = self
					.client
					.get_stream(&path)
					.await
					.map_err(ComposeError::Podman)?;
				let mut stream = if is_tty {
					crate::libpod::parse_raw(resp.into_body())
				} else {
					crate::libpod::parse_multiplexed(resp.into_body())
				};

				while let Some(msg) = stream.next().await {
					match msg.map_err(ComposeError::Podman)? {
						LogOutput::StdOut { message } => {
							print!("{}", String::from_utf8_lossy(&message));
						}
						LogOutput::StdErr { message } => {
							eprint!("{}", String::from_utf8_lossy(&message));
						}
					}
				}
			}
		}

		Ok(())
	}

	/// Run a command in the first replica of the named service. Exits with the command's exit code.
	pub async fn exec(
		&self,
		file: &ComposeFile,
		service_name: &str,
		cmd: Vec<String>,
	) -> Result<()> {
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;
		let container_name = self.first_replica_name(service_name, service);

		let exec_cfg = ExecCreateConfig {
			cmd: Some(cmd),
			attach_stdout: Some(true),
			attach_stderr: Some(true),
			..Default::default()
		};
		let create_path = format!(
			"{API_PREFIX}/containers/{}/exec",
			urlencoded(&container_name),
		);
		let resp: ExecCreateResponse = self
			.client
			.post_json(&create_path, &exec_cfg)
			.await
			.map_err(ComposeError::Podman)?;
		let exec_id = resp.id;

		let start_cfg = ExecStartConfig {
			detach: false,
			tty: false,
		};
		let start_path = format!("{API_PREFIX}/exec/{}/start", urlencoded(&exec_id));
		let start_resp = self
			.client
			.post_json_stream(&start_path, &start_cfg)
			.await
			.map_err(ComposeError::Podman)?;
		let mut stream = crate::libpod::parse_multiplexed(start_resp.into_body());

		while let Some(msg) = stream.next().await {
			match msg.map_err(ComposeError::Podman)? {
				LogOutput::StdOut { message } => {
					print!("{}", String::from_utf8_lossy(&message));
				}
				LogOutput::StdErr { message } => {
					eprint!("{}", String::from_utf8_lossy(&message));
				}
			}
		}

		let inspect_path = format!("{API_PREFIX}/exec/{}/json", urlencoded(&exec_id));
		let inspect: ExecInspect = self
			.client
			.get_json(&inspect_path)
			.await
			.map_err(ComposeError::Podman)?;
		if let Some(code) = inspect.exit_code {
			if code != 0 {
				return Err(ComposeError::RunExited(code));
			}
		}

		Ok(())
	}

	/// Pull images for all services that declare an `image:` key, concurrently.
	pub async fn pull(&self, file: &ComposeFile) -> Result<()> {
		let futs: Vec<_> = file
			.services
			.values()
			.filter(|s| s.image.is_some())
			.map(|s| self.pull_image(s))
			.collect();

		let results = futures_util::future::join_all(futs).await;
		for r in results {
			r?;
		}
		Ok(())
	}

	/// Remove containers labelled for this project that are not defined in the current compose file.
	pub async fn remove_orphans(&self, file: &ComposeFile) -> Result<()> {
		let label = format!("podup.project={}", self.project);
		let filters = serde_json::json!({ "label": [label] });
		let path = format!(
			"{API_PREFIX}/containers/json?all=true&filters={}",
			urlencoded(&filters.to_string()),
		);

		let running = self
			.client
			.get_json::<Vec<crate::libpod::types::container::ContainerListEntry>>(&path)
			.await
			.map_err(ComposeError::Podman)?;

		let known: std::collections::HashSet<String> = file
			.services
			.iter()
			.flat_map(|(n, s)| self.replica_names(n, s))
			.collect();

		for c in running {
			for raw in &c.names {
				let name = raw.trim_start_matches('/');
				if !known.contains(name) {
					tracing::info!("removing orphan container {name}");
					let rm_path =
						format!("{API_PREFIX}/containers/{}?force=true", urlencoded(name));
					if let Err(e) = self.client.delete_ok(&rm_path).await {
						tracing::debug!("orphan delete {name}: {e}");
					}
				}
			}
		}
		Ok(())
	}
}
