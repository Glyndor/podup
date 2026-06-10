//! Query and observation commands: ps, logs, exec, pull, remove_orphans, attach_logs, top, port, images.

use std::collections::HashMap;

use bollard::container::LogOutput;
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::query_parameters::{
	InspectContainerOptions, ListContainersOptions, LogsOptions, RemoveContainerOptions,
};
use futures::StreamExt;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};

use super::Engine;

impl Engine {
	pub async fn ps(&self, _file: &ComposeFile) -> Result<()> {
		let label = format!("podup.project={}", self.project);
		let mut filters: HashMap<String, Vec<String>> = HashMap::new();
		filters.insert("label".to_string(), vec![label]);

		let containers = self
			.docker
			.list_containers(Some(ListContainersOptions {
				all: true,
				filters: Some(filters),
				..Default::default()
			}))
			.await?;

		println!("{:<40} {:<30} {:<20}", "NAME", "IMAGE", "STATUS");
		for c in containers {
			let names = c
				.names
				.unwrap_or_default()
				.join(", ")
				.trim_start_matches('/')
				.to_string();
			let image = c.image.unwrap_or_default();
			let status = c.status.unwrap_or_default();
			let ports = c
				.ports
				.unwrap_or_default()
				.iter()
				.map(|p| {
					format!(
						"{}:{}->{}",
						p.ip.as_deref().unwrap_or(""),
						p.public_port.unwrap_or(0),
						p.private_port
					)
				})
				.collect::<Vec<_>>()
				.join(", ");
			println!("{names:<40} {image:<30} {status:<20} {ports}");
		}

		Ok(())
	}

	pub async fn logs(
		&self,
		file: &ComposeFile,
		service_name: Option<&str>,
		follow: bool,
	) -> Result<()> {
		let targets: Vec<String> = if let Some(svc) = service_name {
			let service = file
				.services
				.get(svc)
				.ok_or_else(|| ComposeError::ServiceNotFound(svc.into()))?;
			vec![self.container_name(svc, service)]
		} else {
			file.services
				.iter()
				.map(|(n, s)| self.container_name(n, s))
				.collect()
		};

		for container_name in targets {
			let mut stream = self.docker.logs(
				&container_name,
				Some(LogsOptions {
					stdout: true,
					stderr: true,
					follow,
					..Default::default()
				}),
			);

			while let Some(msg) = stream.next().await {
				match msg? {
					LogOutput::StdOut { message } => {
						print!("{}", String::from_utf8_lossy(&message));
					}
					LogOutput::StdErr { message } => {
						eprint!("{}", String::from_utf8_lossy(&message));
					}
					_ => {}
				}
			}
		}

		Ok(())
	}

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
		let container_name = self.container_name(service_name, service);

		let exec_id = self
			.docker
			.create_exec(
				&container_name,
				CreateExecOptions::<String> {
					cmd: Some(cmd),
					attach_stdout: Some(true),
					attach_stderr: Some(true),
					attach_stdin: Some(true),
					tty: Some(true),
					..Default::default()
				},
			)
			.await?
			.id;

		match self.docker.start_exec(&exec_id, None).await? {
			StartExecResults::Attached { mut output, .. } => {
				while let Some(msg) = output.next().await {
					match msg? {
						LogOutput::StdOut { message } => {
							print!("{}", String::from_utf8_lossy(&message));
						}
						LogOutput::StdErr { message } => {
							eprint!("{}", String::from_utf8_lossy(&message));
						}
						_ => {}
					}
				}
			}
			StartExecResults::Detached => {}
		}

		Ok(())
	}

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

	pub async fn remove_orphans(&self, file: &ComposeFile) -> Result<()> {
		let label = format!("podup.project={}", self.project);
		let mut filters: HashMap<String, Vec<String>> = HashMap::new();
		filters.insert("label".to_string(), vec![label]);

		let running = self
			.docker
			.list_containers(Some(ListContainersOptions {
				all: true,
				filters: Some(filters),
				..Default::default()
			}))
			.await?;

		let known: std::collections::HashSet<String> = file
			.services
			.iter()
			.flat_map(|(n, s)| self.replica_names(n, s))
			.collect();

		for c in running {
			let names = c.names.unwrap_or_default();
			for raw in &names {
				let name = raw.trim_start_matches('/');
				if !known.contains(name) {
					tracing::info!("removing orphan container {name}");
					let _ = self
						.docker
						.remove_container(
							name,
							Some(RemoveContainerOptions {
								force: true,
								..Default::default()
							}),
						)
						.await;
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
			let container_name = self.container_name(name, service);
			match self.docker.top_processes(&container_name, None).await {
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
		let container_name = self.container_name(service_name, service);

		let info = self
			.docker
			.inspect_container(&container_name, None::<InspectContainerOptions>)
			.await?;

		let key = format!("{private_port}/{proto}");
		let binding = info
			.network_settings
			.and_then(|ns| ns.ports)
			.and_then(|ports| ports.get(&key).cloned().flatten())
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
			match self.docker.inspect_image(&image_ref).await {
				Ok(img) => {
					let (repo, tag) = image_ref
						.rsplit_once(':')
						.map(|(r, t)| (r.to_string(), t.to_string()))
						.unwrap_or_else(|| (image_ref.clone(), "latest".to_string()));
					let id = img
						.id
						.as_deref()
						.unwrap_or("")
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

	pub async fn attach_logs(&self, file: &ComposeFile) -> Result<()> {
		use bollard::query_parameters::LogsOptions;
		use futures::StreamExt;

		let attached: Vec<(String, String)> = file
			.services
			.iter()
			.filter(|(_, s)| s.attach.unwrap_or(true))
			.map(|(name, s)| (name.clone(), self.container_name(name, s)))
			.collect();

		if attached.is_empty() {
			return Ok(());
		}

		let streams: Vec<_> = attached
			.iter()
			.map(|(name, cname)| {
				let prefix = name.clone();
				let mut stream = self.docker.logs(
					cname,
					Some(LogsOptions {
						stdout: true,
						stderr: true,
						follow: true,
						..Default::default()
					}),
				);
				async move {
					while let Some(msg) = stream.next().await {
						match msg {
							Ok(LogOutput::StdOut { message }) => {
								print!("{prefix} | {}", String::from_utf8_lossy(&message));
							}
							Ok(LogOutput::StdErr { message }) => {
								eprint!("{prefix} | {}", String::from_utf8_lossy(&message));
							}
							_ => {}
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
