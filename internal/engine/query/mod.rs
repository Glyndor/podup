//! Query and observation commands: ps, logs, exec, pull, remove_orphans.

use futures_util::StreamExt;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, LogOutput, API_PREFIX};

use super::Engine;
use crate::libpod::types::container::{ContainerListEntry, ContainerPort};

mod exec;
mod inspect;
mod log_prefix;

pub use exec::ExecOptions;
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

/// Options for [`Engine::ps_with_options`], mirroring `docker compose ps`.
#[derive(Default)]
pub struct PsOptions {
	/// Include stopped containers, `-a/--all` (default: running only).
	pub all: bool,
	/// Print only container IDs, `-q/--quiet`.
	pub quiet: bool,
	/// Emit JSON instead of the table, `--format json`.
	pub json: bool,
	/// Print the service names instead of the container table, `--services`.
	pub services_only: bool,
	/// Restrict to these services' containers (positional `SERVICE` filter).
	pub services: Vec<String>,
	/// Status filters, `--status` (e.g. running, exited); OR-combined.
	pub status: Vec<String>,
	/// Generic `KEY=VALUE` predicates, `--filter` (supports status= and name=).
	pub filters: Vec<String>,
}

/// Whether a container status/state word satisfies a `--status`/`status=` filter.
/// Each wanted value matches case-insensitively as a prefix of the status word
/// (so `running` matches `running` and `up`-style strings via the state). An
/// empty `wanted` matches everything. Pure so the predicate is unit-tested.
fn status_matches(status: &str, wanted: &[String]) -> bool {
	if wanted.is_empty() {
		return true;
	}
	let s = status.trim().to_ascii_lowercase();
	wanted.iter().any(|w| {
		let w = w.trim().to_ascii_lowercase();
		!w.is_empty() && (s == w || s.starts_with(&w))
	})
}

/// Split `--filter KEY=VALUE` predicates into the supported buckets: extra
/// `status=` values are folded into the status filter, `name=` values into the
/// name-substring filter, and anything else is returned as `unknown` so the
/// caller can warn. Pure so it is unit-tested.
fn split_ps_filters(filters: &[String]) -> (Vec<String>, Vec<String>, Vec<String>) {
	let (mut status, mut names, mut unknown) = (Vec::new(), Vec::new(), Vec::new());
	for f in filters {
		match f.split_once('=') {
			Some(("status", v)) => status.push(v.to_string()),
			Some(("name", v)) => names.push(v.to_string()),
			_ => unknown.push(f.clone()),
		}
	}
	(status, names, unknown)
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
	/// Produce monochrome output (no colour in the prefix), `--no-color`.
	pub no_color: bool,
	/// Do not print the `{service} | ` prefix, `--no-log-prefix`.
	pub no_log_prefix: bool,
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
	/// (include stopped), `-q/--quiet` (IDs only), `--format` (table | json),
	/// `--services` (service-name list), a positional `SERVICE` filter, and
	/// `--status`/`--filter` predicates.
	pub async fn ps_with_options(&self, file: &ComposeFile, opts: PsOptions) -> Result<()> {
		for name in &opts.services {
			if !file.services.contains_key(name) {
				return Err(ComposeError::ServiceNotFound(name.clone()));
			}
		}

		// `--services` lists the (optionally filtered) configured service names,
		// one per line, instead of the container table.
		if opts.services_only {
			for name in file.services.keys() {
				if opts.services.is_empty() || opts.services.iter().any(|s| s == name) {
					println!("{name}");
				}
			}
			return Ok(());
		}

		// Fold `--status` and any `status=`/`name=` from `--filter` together;
		// warn on unsupported `--filter` keys rather than silently ignoring them.
		let (mut status_filter, name_filter, unknown) = split_ps_filters(&opts.filters);
		for u in &unknown {
			tracing::warn!("ps: ignoring unsupported filter '{u}'");
		}
		status_filter.extend(opts.status.iter().cloned());

		// A positional `SERVICE` filter restricts to those services' container
		// names (across replicas).
		let allowed_names: Option<std::collections::HashSet<String>> = if opts.services.is_empty() {
			None
		} else {
			Some(
				opts.services
					.iter()
					.filter_map(|n| file.services.get(n).map(|s| (n, s)))
					.flat_map(|(n, s)| self.replica_names(n, s))
					.collect(),
			)
		};

		let label = format!("podup.project={}", self.project);
		let filters = serde_json::json!({ "label": [label] });
		let path = format!(
			"{API_PREFIX}/containers/json?all={}&filters={}",
			opts.all,
			urlencoded(&filters.to_string()),
		);

		let all_containers = self
			.client
			.get_json::<Vec<crate::libpod::types::container::ContainerListEntry>>(&path)
			.await
			.map_err(ComposeError::Podman)?;

		let name_of = |c: &crate::libpod::types::container::ContainerListEntry| {
			c.names.join(", ").trim_start_matches('/').to_string()
		};

		let containers: Vec<crate::libpod::types::container::ContainerListEntry> = all_containers
			.into_iter()
			.filter(|c| {
				let name = name_of(c);
				allowed_names.as_ref().is_none_or(|set| {
					c.names
						.iter()
						.any(|n| set.contains(n.trim_start_matches('/')))
				}) && (status_matches(&c.state, &status_filter)
					|| status_matches(&c.status, &status_filter))
					&& (name_filter.is_empty() || name_filter.iter().any(|nf| name.contains(nf)))
			})
			.collect();

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
		let follow = opts.follow;
		// `--no-log-prefix` drops the `{service} | ` tag; `--no-color` forces a
		// monochrome prefix even on a colour-capable stdout.
		let prefix = !opts.no_log_prefix;
		let allow_color = !opts.no_color;
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
						let mut out_pfx = LinePrefixer::new(&container_name, prefix, allow_color);
						let mut err_pfx = LinePrefixer::new(&container_name, prefix, allow_color);
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
				let mut out_pfx = LinePrefixer::new(&container_name, prefix, allow_color);
				let mut err_pfx = LinePrefixer::new(&container_name, prefix, allow_color);
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
		display_status, filter_orphans, format_ports, log_query, split_ps_filters, status_matches,
		LogsOptions,
	};
	use crate::libpod::types::container::{ContainerListEntry, ContainerPort};
	use std::collections::{HashMap, HashSet};

	#[test]
	fn status_matches_empty_filter_matches_all() {
		assert!(status_matches("running", &[]));
		assert!(status_matches("exited", &[]));
	}

	#[test]
	fn status_matches_is_case_insensitive_prefix() {
		assert!(status_matches("running", &["RUNNING".to_string()]));
		assert!(status_matches("exited", &["exit".to_string()]));
		assert!(!status_matches("running", &["exited".to_string()]));
		// An empty wanted value never matches.
		assert!(!status_matches("running", &["".to_string()]));
	}

	#[test]
	fn split_ps_filters_buckets_known_keys_and_flags_unknown() {
		let (status, names, unknown) = split_ps_filters(&[
			"status=running".to_string(),
			"name=web".to_string(),
			"label=foo".to_string(),
		]);
		assert_eq!(status, vec!["running".to_string()]);
		assert_eq!(names, vec!["web".to_string()]);
		assert_eq!(unknown, vec!["label=foo".to_string()]);
	}

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
			..Default::default()
		});
		assert!(q.contains("follow=true"));
		assert!(q.contains("timestamps=true"));
		assert!(q.contains("&tail=20"));
		assert!(q.contains("&since=10m"));
		// `:` is percent-encoded in the query value.
		assert!(q.contains("&until=2024-01-01T00%3A00%3A00"));
	}
}
