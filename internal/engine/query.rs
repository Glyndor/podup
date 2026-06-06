//! Query and observation commands: ps, logs, exec, pull, remove_orphans, attach_logs.

use std::collections::HashMap;

use bollard::container::LogOutput;
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::query_parameters::{ListContainersOptions, LogsOptions, RemoveContainerOptions};
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

		tokio::select! {
			_ = futures::future::join_all(streams) => {}
			_ = tokio::signal::ctrl_c() => {}
		}

		Ok(())
	}
}
