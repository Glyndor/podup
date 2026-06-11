//! Query and observation commands: ps, logs, exec, pull, remove_orphans, attach_logs, top, port, images.

use futures::StreamExt;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::types::exec::{ExecCreateConfig, ExecCreateResponse, ExecInspect, ExecStartConfig};
use crate::libpod::types::image::ImageInspect;
use crate::libpod::{urlencoded, LogOutput};

use super::Engine;

impl Engine {
	/// List containers for this project: name, image, command, state, and port bindings.
	pub async fn ps(&self, _file: &ComposeFile) -> Result<()> {
		let label = format!("podup.project={}", self.project);
		let filters = serde_json::json!({ "label": [label] });
		let path = format!(
			"/libpod/containers/json?all=true&filters={}",
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
					self.replica_names(n, s).into_iter().map(move |cname| (cname, is_tty))
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
							"/libpod/containers/{}/logs?stdout=true&stderr=true&follow=true",
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
			futures::future::join_all(futs).await;
		} else {
			for (container_name, is_tty) in targets {
				let path = format!(
					"/libpod/containers/{}/logs?stdout=true&stderr=true&follow={}",
					urlencoded(&container_name),
					follow,
				);
				let resp = self.client.get_stream(&path).await.map_err(ComposeError::Podman)?;
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
			"/libpod/containers/{}/exec",
			urlencoded(&container_name),
		);
		let resp: ExecCreateResponse = self
			.client
			.post_json(&create_path, &exec_cfg)
			.await
			.map_err(ComposeError::Podman)?;
		let exec_id = resp.id;

		let start_cfg = ExecStartConfig { detach: false, tty: false };
		let start_path = format!("/libpod/exec/{}/start", urlencoded(&exec_id));
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

		let inspect_path = format!("/libpod/exec/{}/json", urlencoded(&exec_id));
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

		let results = futures::future::join_all(futs).await;
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
			"/libpod/containers/json?all=true&filters={}",
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
						format!("/libpod/containers/{}?force=true", urlencoded(name));
					if let Err(e) = self.client.delete_ok(&rm_path).await {
						tracing::debug!("orphan delete {name}: {e}");
					}
				}
			}
		}
		Ok(())
	}

	/// Display running processes in each service container (`docker compose top`).
	///
	/// If `target_services` is empty, all services are queried.
	pub async fn top(&self, file: &ComposeFile, target_services: &[String]) -> Result<()> {
		let names: Vec<String> = if target_services.is_empty() {
			file.services.keys().cloned().collect()
		} else {
			for name in target_services {
				if !file.services.contains_key(name) {
					return Err(crate::error::ComposeError::ServiceNotFound(name.clone()));
				}
			}
			target_services.to_vec()
		};

		for name in &names {
			let service = &file.services[name];
			for container_name in self.replica_names(name, service) {
				let path = format!(
					"/libpod/containers/{}/top",
					urlencoded(&container_name),
				);
				match self
					.client
					.get_json::<crate::libpod::types::container::TopResponse>(&path)
					.await
				{
					Ok(result) => {
						println!("{container_name}");
						if let Some(titles) = &result.titles {
							println!("{}", titles.join("\t"));
						}
						if let Some(processes) = &result.processes {
							for row in processes {
								println!("{}", row.join("\t"));
							}
						}
					}
					Err(e) => tracing::warn!("top {container_name}: {e}"),
				}
			}
		}
		Ok(())
	}

	/// Print the public port for a given private port of a service container.
	///
	/// `proto` should be `"tcp"` or `"udp"`. Prints `HOST:PORT` to stdout.
	pub async fn port(
		&self,
		file: &ComposeFile,
		service_name: &str,
		private_port: u16,
		proto: &str,
	) -> Result<()> {
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| crate::error::ComposeError::ServiceNotFound(service_name.into()))?;
		let container_name = self.first_replica_name(service_name, service);

		let path = format!(
			"/libpod/containers/{}/json",
			urlencoded(&container_name),
		);
		let info = self
			.client
			.get_json::<crate::libpod::types::container::ContainerInspect>(&path)
			.await
			.map_err(ComposeError::Podman)?;

		let key = format!("{private_port}/{proto}");
		let binding = info
			.network_settings
			.and_then(|ns| ns.ports.get(&key).cloned().flatten())
			.and_then(|bindings| bindings.into_iter().next());

		match binding {
			Some(b) => {
				let host = b.host_ip.as_deref().unwrap_or("0.0.0.0");
				let port = b.host_port.as_deref().unwrap_or("");
				println!("{host}:{port}");
			}
			None => println!(),
		}
		Ok(())
	}

	/// List images used by each service.
	pub async fn images(&self, file: &ComposeFile) -> Result<()> {
		println!(
			"{:<30} {:<25} {:<15} {:<20}",
			"SERVICE", "REPOSITORY", "TAG", "IMAGE ID"
		);
		for (name, service) in &file.services {
			let image_ref = match &service.image {
				Some(img) => img.clone(),
				None if service.build.is_some() => format!("{name}:latest"),
				None => continue,
			};
			let path = format!("/libpod/images/{}/json", urlencoded(&image_ref));
			match self.client.get_json::<ImageInspect>(&path).await {
				Ok(img) => {
					let (repo, tag) = image_ref
						.rsplit_once(':')
						.map(|(r, t)| (r.to_string(), t.to_string()))
						.unwrap_or_else(|| (image_ref.clone(), "latest".to_string()));
					let id = img
						.id
						.trim_start_matches("sha256:")
						.get(..12)
						.unwrap_or("");
					println!("{name:<30} {repo:<25} {tag:<15} {id:<20}");
				}
				Err(e) => tracing::warn!("images {name}: {e}"),
			}
		}
		Ok(())
	}

	/// Attach to log streams for all services with `attach: true` (the default). Streams are multiplexed to stdout with a service-name prefix.
	pub async fn attach_logs(&self, file: &ComposeFile) -> Result<()> {
		// Carry (display_name, container_name, is_tty) so the log parser matches
		// the container's framing mode: TTY containers emit raw bytes; non-TTY
		// containers emit multiplexed 8-byte-header frames.
		let attached: Vec<(String, String, bool)> = file
			.services
			.iter()
			.filter(|(_, s)| s.attach.unwrap_or(true))
			.flat_map(|(name, s)| {
				let proj_prefix = format!("{}-", self.project);
				let is_tty = s.tty.unwrap_or(false);
				self.replica_names(name, s).into_iter().map(move |cname| {
					let display = cname
						.strip_prefix(proj_prefix.as_str())
						.map(|s| s.to_string())
						.unwrap_or_else(|| cname.clone());
					(display, cname, is_tty)
				})
			})
			.collect();

		if attached.is_empty() {
			return Ok(());
		}

		let streams: Vec<_> = attached
			.iter()
			.map(|(display, cname, is_tty)| {
				let prefix = display.clone();
				let path = format!(
					"/libpod/containers/{}/logs?stdout=true&stderr=true&follow=true",
					urlencoded(cname),
				);
				let client = &self.client;
				let is_tty = *is_tty;
				async move {
					let resp = match client.get_stream(&path).await {
						Ok(r) => r,
						Err(e) => {
							tracing::warn!("attach_logs {prefix}: {e}");
							return;
						}
					};
					// TTY containers produce raw bytes (stdout/stderr merged).
					// Non-TTY containers produce multiplexed frames with 8-byte headers.
					let mut stream = if is_tty {
						crate::libpod::parse_raw(resp.into_body())
					} else {
						crate::libpod::parse_multiplexed(resp.into_body())
					};
					while let Some(msg) = stream.next().await {
						match msg {
							Ok(LogOutput::StdOut { message }) => {
								print!("{prefix} | {}", String::from_utf8_lossy(&message));
							}
							Ok(LogOutput::StdErr { message }) => {
								eprint!("{prefix} | {}", String::from_utf8_lossy(&message));
							}
							Err(_) => break,
						}
					}
				}
			})
			.collect();

		#[cfg(unix)]
		{
			use tokio::signal::unix::{signal, SignalKind};
			let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM handler");
			tokio::select! {
				_ = futures::future::join_all(streams) => {}
				_ = tokio::signal::ctrl_c() => {}
				_ = sigterm.recv() => {}
			}
		}
		#[cfg(not(unix))]
		tokio::select! {
			_ = futures::future::join_all(streams) => {}
			_ = tokio::signal::ctrl_c() => {}
		}

		Ok(())
	}
}
