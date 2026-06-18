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

/// Options for [`Engine::exec`], mirroring `docker compose exec` flags.
#[derive(Default)]
pub struct ExecOptions {
	/// Extra environment variables (`KEY=VAL`), `-e/--env`.
	pub env: Vec<String>,
	/// Run as this user, `-u/--user`.
	pub user: Option<String>,
	/// Working directory inside the container, `-w/--workdir`.
	pub workdir: Option<String>,
	/// Run with extended privileges, `--privileged`.
	pub privileged: bool,
	/// Detach: start the exec and return without streaming output, `-d/--detach`.
	pub detach: bool,
	/// 1-based replica index for a scaled service, `--index` (default: first).
	pub index: Option<u32>,
}

/// Options for [`Engine::ps_with_options`].
#[derive(Default)]
pub struct PsOptions {
	/// Include stopped containers, `-a/--all` (default: running only).
	pub all: bool,
	/// Print only container IDs, `-q/--quiet`.
	pub quiet: bool,
	/// Emit JSON instead of the table, `--format json`.
	pub json: bool,
}

/// Options for [`Engine::images_with_options`].
#[derive(Default)]
pub struct ImagesOptions {
	/// Print only image IDs, `-q/--quiet`.
	pub quiet: bool,
	/// Emit JSON instead of the table, `--format json`.
	pub json: bool,
}

/// Options for [`Engine::logs_with_options`], mirroring `docker compose logs`.
#[derive(Default)]
pub struct LogsOptions {
	/// Follow log output, `-f/--follow`.
	pub follow: bool,
	/// Number of lines to show from the end, `-n/--tail` (`None` = all).
	pub tail: Option<String>,
	/// Show logs since a timestamp/relative time, `--since`.
	pub since: Option<String>,
	/// Show logs until a timestamp/relative time, `--until`.
	pub until: Option<String>,
	/// Prefix each line with an RFC3339 timestamp, `-t/--timestamps`.
	pub timestamps: bool,
}

/// Build the libpod `containers/{}/logs` query string from the options.
fn log_query(opts: &LogsOptions) -> String {
	let mut q = format!(
		"stdout=true&stderr=true&follow={}&timestamps={}",
		opts.follow, opts.timestamps
	);
	if let Some(tail) = &opts.tail {
		q.push_str(&format!("&tail={}", urlencoded(tail)));
	}
	if let Some(since) = &opts.since {
		q.push_str(&format!("&since={}", urlencoded(since)));
	}
	if let Some(until) = &opts.until {
		q.push_str(&format!("&until={}", urlencoded(until)));
	}
	q
}

impl Engine {
	/// List running containers for this project as a table (default options).
	pub async fn ps(&self, file: &ComposeFile) -> Result<()> {
		self.ps_with_options(file, PsOptions::default()).await
	}

	/// List containers with `docker compose ps`-style options: `-a/--all`
	/// (include stopped), `-q/--quiet` (IDs only), and `--format` (table | json).
	pub async fn ps_with_options(&self, _file: &ComposeFile, opts: PsOptions) -> Result<()> {
		let label = format!("podup.project={}", self.project);
		let filters = serde_json::json!({ "label": [label] });
		let path = format!(
			"{API_PREFIX}/containers/json?all={}&filters={}",
			opts.all,
			urlencoded(&filters.to_string()),
		);

		let containers = self
			.client
			.get_json::<Vec<crate::libpod::types::container::ContainerListEntry>>(&path)
			.await
			.map_err(ComposeError::Podman)?;

		let name_of = |c: &crate::libpod::types::container::ContainerListEntry| {
			c.names.join(", ").trim_start_matches('/').to_string()
		};

		if opts.quiet {
			for c in &containers {
				let id = c.id.get(..12).unwrap_or(&c.id);
				println!("{id}");
			}
			return Ok(());
		}

		if opts.json {
			let rows: Vec<_> = containers
				.iter()
				.map(|c| {
					serde_json::json!({
						"Name": name_of(c),
						"Image": c.image,
						"Status": c.status,
						"ID": c.id,
					})
				})
				.collect();
			println!(
				"{}",
				serde_json::to_string_pretty(&rows).unwrap_or_default()
			);
			return Ok(());
		}

		println!("{:<40} {:<30} {:<20}", "NAME", "IMAGE", "STATUS");
		for c in &containers {
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
			println!(
				"{:<40} {:<30} {:<20} {ports}",
				name_of(c),
				c.image,
				c.status
			);
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
		self.logs_with_options(
			file,
			service_name,
			LogsOptions {
				follow,
				..Default::default()
			},
		)
		.await
	}

	/// Stream logs with `docker compose logs` options (`--tail`, `--since`,
	/// `--until`, `--timestamps`, `--follow`).
	pub async fn logs_with_options(
		&self,
		file: &ComposeFile,
		service_name: Option<&str>,
		opts: LogsOptions,
	) -> Result<()> {
		let follow = opts.follow;
		let query = log_query(&opts);
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
					let query = query.clone();
					async move {
						let path = format!(
							"{API_PREFIX}/containers/{}/logs?{query}",
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
					"{API_PREFIX}/containers/{}/logs?{query}",
					urlencoded(&container_name),
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

	/// Run a command in the first replica of the named service with default
	/// options. Exits with the command's exit code.
	pub async fn exec(
		&self,
		file: &ComposeFile,
		service_name: &str,
		cmd: Vec<String>,
	) -> Result<()> {
		self.exec_with_options(file, service_name, cmd, ExecOptions::default())
			.await
	}

	/// Run a command in a service container with `docker compose exec`-style
	/// overrides (env, user, workdir, privileged, detach, replica index).
	pub async fn exec_with_options(
		&self,
		file: &ComposeFile,
		service_name: &str,
		cmd: Vec<String>,
		opts: ExecOptions,
	) -> Result<()> {
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;
		let container_name = match opts.index {
			Some(i) => {
				let names = self.replica_names(service_name, service);
				let idx = (i as usize).saturating_sub(1);
				names.get(idx).cloned().ok_or_else(|| {
					ComposeError::ServiceNotFound(format!("{service_name} (replica index {i})"))
				})?
			}
			None => self.first_replica_name(service_name, service),
		};

		let exec_cfg = ExecCreateConfig {
			cmd: Some(cmd),
			attach_stdout: Some(true),
			attach_stderr: Some(true),
			user: opts.user.clone(),
			working_dir: opts.workdir.clone(),
			privileged: opts.privileged.then_some(true),
			env: (!opts.env.is_empty()).then(|| opts.env.clone()),
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

		// `-d/--detach`: start the exec and return without streaming output or
		// waiting for the exit code. The server returns immediately, so the
		// response body is dropped.
		if opts.detach {
			let start_cfg = ExecStartConfig {
				detach: true,
				tty: false,
			};
			let start_path = format!("{API_PREFIX}/exec/{}/start", urlencoded(&exec_id));
			let _ = self
				.client
				.post_json_stream(&start_path, &start_cfg)
				.await
				.map_err(ComposeError::Podman)?;
			return Ok(());
		}

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

#[cfg(test)]
mod tests {
	use super::{log_query, LogsOptions};

	#[test]
	fn log_query_defaults_to_stdout_stderr_no_follow() {
		let q = log_query(&LogsOptions::default());
		assert_eq!(q, "stdout=true&stderr=true&follow=false&timestamps=false");
	}

	#[test]
	fn log_query_includes_set_options() {
		let q = log_query(&LogsOptions {
			follow: true,
			tail: Some("20".into()),
			since: Some("10m".into()),
			until: Some("2024-01-01T00:00:00".into()),
			timestamps: true,
		});
		assert!(q.contains("follow=true"));
		assert!(q.contains("timestamps=true"));
		assert!(q.contains("&tail=20"));
		assert!(q.contains("&since=10m"));
		// `:` is percent-encoded in the query value.
		assert!(q.contains("&until=2024-01-01T00%3A00%3A00"));
	}
}
