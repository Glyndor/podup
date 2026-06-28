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
use crate::libpod::types::container::{ContainerListEntry, ContainerPort};

mod inspect;
mod log_prefix;

use log_prefix::LinePrefixer;

/// Human-readable status for `ps`. Podman's libpod list endpoint leaves
/// `Status` empty and reports the machine state in `State`, so fall back to it
/// rather than rendering a blank column.
fn display_status(c: &ContainerListEntry) -> &str {
	if c.status.is_empty() {
		&c.state
	} else {
		&c.status
	}
}

/// Render a container's published ports the way `docker compose ps` does, e.g.
/// `0.0.0.0:8080->80/tcp`. An unset host IP means "all interfaces", shown as
/// `0.0.0.0` (libpod commonly omits it) to match Docker/Podman output.
fn format_ports(ports: &[ContainerPort]) -> String {
	ports
		.iter()
		.map(|p| {
			let proto = p
				.protocol
				.as_deref()
				.map(|proto| format!("/{proto}"))
				.unwrap_or_default();
			let host_ip = p
				.host_ip
				.as_deref()
				.filter(|s| !s.is_empty())
				.unwrap_or("0.0.0.0");
			format!(
				"{host_ip}:{}->{}{proto}",
				p.host_port.unwrap_or(0),
				p.container_port
			)
		})
		.collect::<Vec<_>>()
		.join(", ")
}

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

/// Map a libpod error from an `exec`/`attach` target into a friendly
/// [`ComposeError::NotRunning`] when it means the container is absent (404) or
/// stopped ("can only create exec sessions on running containers"), so the user
/// sees "service X is not running" instead of a raw HTTP 404/500. Any other
/// failure passes through unchanged. Pure so it is unit-tested.
fn map_not_running(e: crate::libpod::PodmanError, service_name: &str) -> ComposeError {
	let not_running = e.is_status(404)
		|| matches!(
			&e,
			crate::libpod::PodmanError::Api { message, .. }
				if {
					let m = message.to_ascii_lowercase();
					m.contains("can only create exec sessions on running containers")
						|| m.contains("is not running")
						|| m.contains("no such container")
				}
		);
	if not_running {
		ComposeError::NotRunning(service_name.to_string())
	} else {
		ComposeError::Podman(e)
	}
}

/// Validate the `--tail`/`--since`/`--until` values client-side so a typo is
/// rejected with a clear local message instead of a raw podman HTTP 400. `tail`
/// must be `all` or a non-negative integer; `since`/`until` must be a Unix
/// timestamp or a Go-style duration (e.g. `10m`, `1h30m`) or an RFC3339-ish
/// timestamp. Pure so it is unit-tested.
fn validate_log_filters(opts: &LogsOptions) -> Result<()> {
	if let Some(tail) = &opts.tail {
		if tail != "all" && tail.parse::<u64>().is_err() {
			return Err(ComposeError::Unsupported(format!(
				"invalid --tail value {tail:?}: expected a non-negative integer or 'all'"
			)));
		}
	}
	for (flag, value) in [("--since", &opts.since), ("--until", &opts.until)] {
		if let Some(v) = value {
			if !is_valid_log_time(v) {
				return Err(ComposeError::Unsupported(format!(
					"invalid {flag} value {v:?}: expected a duration (e.g. 10m, 1h30m), a Unix \
					 timestamp, or an RFC3339 time"
				)));
			}
		}
	}
	Ok(())
}

/// Whether a `--since`/`--until` value is a plausible duration, Unix timestamp,
/// or timestamp string. Conservative: rejects obvious garbage (`abc`) while
/// accepting the forms podman understands.
fn is_valid_log_time(v: &str) -> bool {
	if v.is_empty() {
		return false;
	}
	// Unix timestamp (optionally fractional).
	if v.parse::<f64>().is_ok() {
		return true;
	}
	// Go-style duration: digit-run + unit, repeated (e.g. 1h30m, 90s, 500ms).
	if is_go_duration(v) {
		return true;
	}
	// Timestamp-ish: starts with a 4-digit year and contains only the characters
	// an RFC3339/date string uses. The server does the precise parse; this just
	// blocks free-form garbage.
	let bytes = v.as_bytes();
	bytes.len() >= 4
		&& bytes[..4].iter().all(u8::is_ascii_digit)
		&& v.chars().all(|c| {
			c.is_ascii_digit() || matches!(c, '-' | ':' | 't' | 'T' | 'z' | 'Z' | '.' | '+' | ' ')
		})
}

/// Match a Go-style duration: one or more `<number><unit>` segments, units one
/// of `ns,us,µs,ms,s,m,h`.
fn is_go_duration(v: &str) -> bool {
	let mut rest = v.strip_prefix('-').unwrap_or(v);
	if rest.is_empty() {
		return false;
	}
	let mut segments = 0;
	while !rest.is_empty() {
		let digits = rest.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
		if digits.len() == rest.len() {
			// No digits consumed → not a duration segment.
			return false;
		}
		rest = digits;
		let unit_len = ["ms", "ns", "us", "µs", "s", "m", "h"]
			.into_iter()
			.find(|u| rest.starts_with(u))
			.map(str::len);
		match unit_len {
			Some(n) => rest = &rest[n..],
			None => return false,
		}
		segments += 1;
	}
	segments > 0
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
						"Status": display_status(c),
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

		crate::ui::print_bold_header(&format!(
			"{:<40} {:<30} {:<20} PORTS",
			"NAME", "IMAGE", "STATUS"
		));
		for c in &containers {
			let ports = format_ports(&c.ports);
			let status = crate::ui::status_cell(display_status(c), 20);
			println!("{:<40} {:<30} {status} {ports}", name_of(c), c.image);
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
		validate_log_filters(&opts)?;
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
				// Tolerate a missing/not-yet-created container the way the
				// multi-follow path does: warn and move on so the logs of the
				// services that *do* exist are still shown, instead of aborting the
				// whole command on the first 404.
				let resp = match self.client.get_stream(&path).await {
					Ok(r) => r,
					Err(e) => {
						tracing::warn!("logs {container_name}: {e}");
						continue;
					}
				};
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
					match msg {
						Ok(LogOutput::StdOut { message }) => out_pfx.write(&mut out, &message),
						Ok(LogOutput::StdErr { message }) => {
							err_pfx.write(&mut std::io::stderr().lock(), &message)
						}
						Err(_) => break,
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
		if cmd.is_empty() {
			return Err(ComposeError::Unsupported(format!(
				"exec into service '{service_name}' requires a command to run"
			)));
		}
		let container_name = self.replica_name_at(service_name, service, opts.index)?;

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
			.map_err(|e| map_not_running(e, service_name))?;
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
	use super::{
		display_status, filter_orphans, format_ports, is_valid_log_time, log_query,
		map_not_running, validate_log_filters, LogsOptions,
	};
	use crate::libpod::types::container::{ContainerListEntry, ContainerPort};
	use std::collections::{HashMap, HashSet};

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

	fn entry(status: &str, state: &str) -> ContainerListEntry {
		ContainerListEntry {
			id: "abc123".into(),
			names: vec!["/web".into()],
			image: "alpine".into(),
			status: status.into(),
			state: state.into(),
			ports: vec![],
			labels: HashMap::new(),
		}
	}

	#[test]
	fn display_status_falls_back_to_state_when_status_empty() {
		// Podman 5's libpod list endpoint sends an empty `Status` and the real
		// machine state in `State` — `ps` must show the latter, not a blank.
		assert_eq!(display_status(&entry("", "running")), "running");
		assert_eq!(display_status(&entry("", "exited")), "exited");
	}

	#[test]
	fn display_status_prefers_status_when_present() {
		assert_eq!(
			display_status(&entry("Up 2 seconds", "running")),
			"Up 2 seconds"
		);
	}

	#[test]
	fn format_ports_defaults_missing_host_ip_to_all_interfaces() {
		let p = ContainerPort {
			host_ip: None,
			host_port: Some(8080),
			container_port: 80,
			protocol: Some("tcp".into()),
			..Default::default()
		};
		assert_eq!(
			format_ports(std::slice::from_ref(&p)),
			"0.0.0.0:8080->80/tcp"
		);
	}

	#[test]
	fn format_ports_keeps_explicit_host_ip() {
		let p = ContainerPort {
			host_ip: Some("127.0.0.1".into()),
			host_port: Some(5432),
			container_port: 5432,
			..Default::default()
		};
		assert_eq!(
			format_ports(std::slice::from_ref(&p)),
			"127.0.0.1:5432->5432"
		);
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

	#[test]
	fn validate_log_filters_accepts_good_values() {
		assert!(validate_log_filters(&LogsOptions {
			tail: Some("all".into()),
			since: Some("10m".into()),
			until: Some("2024-01-01T00:00:00Z".into()),
			..Default::default()
		})
		.is_ok());
		assert!(validate_log_filters(&LogsOptions {
			tail: Some("100".into()),
			since: Some("1700000000".into()),
			..Default::default()
		})
		.is_ok());
		assert!(validate_log_filters(&LogsOptions::default()).is_ok());
	}

	#[test]
	fn validate_log_filters_rejects_bad_tail_and_time() {
		assert!(validate_log_filters(&LogsOptions {
			tail: Some("abc".into()),
			..Default::default()
		})
		.is_err());
		assert!(validate_log_filters(&LogsOptions {
			since: Some("yesterday".into()),
			..Default::default()
		})
		.is_err());
		assert!(validate_log_filters(&LogsOptions {
			until: Some("not-a-time".into()),
			..Default::default()
		})
		.is_err());
	}

	#[test]
	fn is_valid_log_time_classifies_forms() {
		assert!(is_valid_log_time("10m"));
		assert!(is_valid_log_time("1h30m"));
		assert!(is_valid_log_time("500ms"));
		assert!(is_valid_log_time("1700000000"));
		assert!(is_valid_log_time("2024-01-02T03:04:05Z"));
		assert!(!is_valid_log_time("abc"));
		assert!(!is_valid_log_time(""));
		assert!(!is_valid_log_time("10x"));
	}

	#[test]
	fn map_not_running_maps_404_and_stopped() {
		use crate::error::ComposeError;
		use crate::libpod::PodmanError;
		let e404 = PodmanError::Api {
			status: 404,
			message: "no such container: web".into(),
		};
		assert!(matches!(
			map_not_running(e404, "web"),
			ComposeError::NotRunning(s) if s == "web"
		));
		let e500 = PodmanError::Api {
			status: 500,
			message: "can only create exec sessions on running containers".into(),
		};
		assert!(matches!(
			map_not_running(e500, "web"),
			ComposeError::NotRunning(_)
		));
		// An unrelated error passes through unchanged.
		let other = PodmanError::Api {
			status: 500,
			message: "disk full".into(),
		};
		assert!(matches!(
			map_not_running(other, "web"),
			ComposeError::Podman(_)
		));
	}
}
