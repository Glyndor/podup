//! Query and observation commands: ps, logs, exec, pull, remove_orphans.

use std::io::Write;

use futures_util::StreamExt;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::types::exec::{
	ExecCreateConfig, ExecCreateResponse, ExecInspect, ExecStartConfig,
};
use crate::libpod::{urlencoded, LogOutput, API_PREFIX};

use super::Engine;

mod inspect;
mod inspect_util;
mod log_prefix;
mod ps;

pub use ps::PsOptions;

use log_prefix::LinePrefixer;

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
	/// Stream logs. When `service_name` is `None`, streams from all services. When `follow` is true, tails indefinitely.
	pub async fn logs(
		&self,
		file: &ComposeFile,
		service_name: Option<&str>,
		follow: bool,
	) -> Result<()> {
		let targets: Vec<String> = service_name
			.map(|s| vec![s.to_string()])
			.unwrap_or_default();
		self.logs_with_options(
			file,
			&targets,
			LogsOptions {
				follow,
				..Default::default()
			},
		)
		.await
	}

	/// Stream logs with `docker compose logs` options (`--tail`, `--since`,
	/// `--until`, `--timestamps`, `--follow`).
	///
	/// When `target_services` is empty, logs from every service are streamed;
	/// otherwise only the named services (an unknown name is an error).
	pub async fn logs_with_options(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		opts: LogsOptions,
	) -> Result<()> {
		let follow = opts.follow;
		let query = log_query(&opts);
		for svc in target_services {
			if !file.services.contains_key(svc) {
				return Err(ComposeError::ServiceNotFound(svc.into()));
			}
		}
		let selected: std::collections::HashSet<&str> =
			target_services.iter().map(String::as_str).collect();
		// (container_name, is_tty) — TTY containers send raw bytes; non-TTY use
		// multiplexed 8-byte-header framing.
		let targets: Vec<(String, bool)> = file
			.services
			.iter()
			.filter(|(n, _)| selected.is_empty() || selected.contains(n.as_str()))
			.flat_map(|(n, s)| {
				let is_tty = s.tty.unwrap_or(false);
				self.replica_names(n, s)
					.into_iter()
					.map(move |cname| (cname, is_tty))
			})
			.collect();

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
						// These futures run concurrently under `join_all` on the
						// same task, so the stdout/stderr lock is taken and
						// released within each frame rather than held across the
						// `.await` above — holding a guard across the await would
						// let a sibling future block the thread on the same lock
						// and deadlock. Each frame still locks once and flushes,
						// keeping interleaved `logs -f` output prompt.
						let mut out_pfx = LinePrefixer::new(&container_name);
						let mut err_pfx = LinePrefixer::new(&container_name);
						while let Some(msg) = stream.next().await {
							match msg {
								Ok(LogOutput::StdOut { message }) => {
									out_pfx.write(&mut std::io::stdout().lock(), &message);
								}
								Ok(LogOutput::StdErr { message }) => {
									err_pfx.write(&mut std::io::stderr().lock(), &message);
								}
								Err(_) => break,
							}
						}
						out_pfx.flush_tail(&mut std::io::stdout().lock());
						err_pfx.flush_tail(&mut std::io::stderr().lock());
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

				// Lock stdout once for the whole stream instead of re-acquiring
				// the lock (and issuing a syscall) per frame; stdout is ours
				// exclusively on this path. stderr is locked per frame because
				// the tracing subscriber also writes there: holding its lock
				// across the await loop would starve concurrent log emissions.
				// Flush after each frame so `logs -f` still streams promptly.
				let mut out = std::io::stdout().lock();
				let mut out_pfx = LinePrefixer::new(&container_name);
				let mut err_pfx = LinePrefixer::new(&container_name);
				while let Some(msg) = stream.next().await {
					match msg.map_err(ComposeError::Podman)? {
						LogOutput::StdOut { message } => out_pfx.write(&mut out, &message),
						LogOutput::StdErr { message } => {
							err_pfx.write(&mut std::io::stderr().lock(), &message)
						}
					}
				}
				out_pfx.flush_tail(&mut out);
				err_pfx.flush_tail(&mut std::io::stderr().lock());
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
		let container_name = self
			.live_replica_name_at(service_name, service, opts.index)
			.await?;

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

		// Lock stdout once for the whole stream instead of re-acquiring the lock
		// (and issuing a syscall) per frame; stdout is ours exclusively on this
		// path. stderr is locked per frame because the tracing subscriber also
		// writes there: holding its lock across the await loop would starve
		// concurrent log emissions. Flush after each frame so exec streams
		// promptly.
		{
			let mut out = std::io::stdout().lock();
			while let Some(msg) = stream.next().await {
				match msg.map_err(ComposeError::Podman)? {
					LogOutput::StdOut { message } => {
						let _ = out.write_all(String::from_utf8_lossy(&message).as_bytes());
						let _ = out.flush();
					}
					LogOutput::StdErr { message } => {
						let mut err = std::io::stderr().lock();
						let _ = err.write_all(String::from_utf8_lossy(&message).as_bytes());
						let _ = err.flush();
					}
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

	/// Names of this project's containers (by label) that the current compose file
	/// no longer defines — the orphans, shared by removal and the warning.
	async fn orphan_container_names(&self, file: &ComposeFile) -> Result<Vec<String>> {
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

		let names: Vec<String> = running
			.iter()
			.flat_map(|c| c.names.iter())
			.map(|raw| raw.trim_start_matches('/').to_string())
			.collect();
		Ok(filter_orphans(names, &known))
	}

	/// Remove containers labelled for this project that are not defined in the current compose file.
	pub async fn remove_orphans(&self, file: &ComposeFile) -> Result<()> {
		for name in self.orphan_container_names(file).await? {
			tracing::info!("removing orphan container {name}");
			let rm_path = format!("{API_PREFIX}/containers/{}?force=true", urlencoded(&name));
			if let Err(e) = self.client.delete_ok(&rm_path).await {
				tracing::debug!("orphan delete {name}: {e}");
			}
		}
		Ok(())
	}

	/// Warn (without removing) when this project has orphan containers and
	/// `--remove-orphans` was not given, matching docker compose's `up`.
	pub async fn warn_orphans(&self, file: &ComposeFile) -> Result<()> {
		let orphans = self.orphan_container_names(file).await?;
		if !orphans.is_empty() {
			eprintln!(
				"Found orphan container(s) ({}) for this project. If you removed or renamed a \
				 service in your compose file, run with --remove-orphans to remove them.",
				orphans.join(", ")
			);
		}
		Ok(())
	}
}

/// The subset of `names` not present in `known` (the orphan containers). Pure so
/// the membership logic is unit-tested without a live Podman socket.
fn filter_orphans(names: Vec<String>, known: &std::collections::HashSet<String>) -> Vec<String> {
	names.into_iter().filter(|n| !known.contains(n)).collect()
}

#[cfg(test)]
mod tests {
	use super::{filter_orphans, log_query, LogsOptions};
	use std::collections::HashSet;

	#[test]
	fn filter_orphans_keeps_only_unknown_names() {
		let known: HashSet<String> = ["web-1".to_string(), "db".to_string()].into();
		let names = vec![
			"web-1".to_string(),
			"db".to_string(),
			"old-cache".to_string(),
		];
		assert_eq!(filter_orphans(names, &known), vec!["old-cache".to_string()]);
	}

	#[test]
	fn filter_orphans_empty_when_all_known() {
		let known: HashSet<String> = ["web".to_string()].into();
		assert!(filter_orphans(vec!["web".to_string()], &known).is_empty());
	}

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
