//! `ps` — list this project's containers as a table or JSON. Split out of the
//! query root so each file stays within the source line limit.

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::types::container::{ContainerListEntry, ContainerPort};
use crate::libpod::{urlencoded, API_PREFIX};

use super::Engine;

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

/// The container's display name (leading slash stripped).
fn name_of(c: &ContainerListEntry) -> String {
	c.names.join(", ").trim_start_matches('/').to_string()
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

/// The health word embedded in a human status string (`Up 2 minutes (healthy)`),
/// or `""` when absent. The libpod list endpoint carries no separate health
/// field, so `ps` derives it from the status text the way `docker ps` shows it.
fn health_from_status(status: &str) -> &'static str {
	let s = status.to_ascii_lowercase();
	if s.contains("unhealthy") {
		"unhealthy"
	} else if s.contains("healthy") {
		"healthy"
	} else if s.contains("health: starting") || s.contains("starting") {
		"starting"
	} else {
		""
	}
}

/// Number of consecutive ports a record covers (`range`, at least 1).
fn span_len(p: &ContainerPort) -> u16 {
	p.range.filter(|&r| r > 0).unwrap_or(1)
}

/// Host IP for display: an unset/empty value means all interfaces (`0.0.0.0`),
/// matching Docker/Podman output (libpod commonly omits it).
fn display_host_ip(p: &ContainerPort) -> &str {
	p.host_ip
		.as_deref()
		.filter(|s| !s.is_empty())
		.unwrap_or("0.0.0.0")
}

/// Render one port record the way `docker compose ps` does. A collapsed range
/// (`range > 1`) is rendered as `host_start-host_end->cont_start-cont_end` so the
/// whole range is shown rather than only its first mapping.
fn format_port_record(p: &ContainerPort) -> String {
	let proto = p
		.protocol
		.as_deref()
		.map(|proto| format!("/{proto}"))
		.unwrap_or_default();
	let host_ip = display_host_ip(p);
	let hp = p.host_port.unwrap_or(0);
	let n = span_len(p);
	if n > 1 {
		format!(
			"{host_ip}:{hp}-{}->{}-{}{proto}",
			hp + n - 1,
			p.container_port,
			p.container_port + n - 1,
		)
	} else {
		format!("{host_ip}:{hp}->{}{proto}", p.container_port)
	}
}

/// Render a container's published ports as a comma-joined `ps` PORTS cell.
fn format_ports(ports: &[ContainerPort]) -> String {
	ports
		.iter()
		.map(format_port_record)
		.collect::<Vec<_>>()
		.join(", ")
}

/// Structured publishers for `ps --format json`, one object per published port
/// (a collapsed range is expanded so every port appears), mirroring the
/// `Publishers` array docker compose emits.
fn publishers(ports: &[ContainerPort]) -> Vec<serde_json::Value> {
	let mut out = Vec::new();
	for p in ports {
		let n = span_len(p);
		for i in 0..n {
			out.push(serde_json::json!({
				"URL": display_host_ip(p),
				"TargetPort": p.container_port + i,
				"PublishedPort": p.host_port.map(|hp| hp + i),
				"Protocol": p.protocol.as_deref().unwrap_or("tcp"),
			}));
		}
	}
	out
}

/// Build one `ps --format json` row, surfacing the fields docker compose
/// machine consumers expect (Service/State/Health/ExitCode/Publishers) in
/// addition to Name/Image/Status/ID. Pure so it can be unit-tested.
fn ps_json_row(c: &ContainerListEntry) -> serde_json::Value {
	serde_json::json!({
		"Name": name_of(c),
		"Image": c.image,
		"Project": c.labels.get("podup.project").cloned().unwrap_or_default(),
		"Service": c.labels.get("podup.service").cloned().unwrap_or_default(),
		"State": c.state,
		"Status": display_status(c),
		"Health": health_from_status(display_status(c)),
		"ExitCode": c.exit_code.unwrap_or(0),
		"Publishers": publishers(&c.ports),
		"ID": c.id,
	})
}

impl Engine {
	/// List running containers for this project as a table (default options).
	pub async fn ps(&self, file: &ComposeFile) -> Result<()> {
		self.ps_with_options(file, PsOptions::default()).await
	}

	/// List containers with `docker compose ps`-style options: `-a/--all`
	/// (include stopped), `-q/--quiet` (full IDs only), `--format`
	/// (table | json), `--services` (service-name list), a positional `SERVICE`
	/// filter, and `--status`/`--filter` predicates.
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
			.get_json::<Vec<ContainerListEntry>>(&path)
			.await
			.map_err(ComposeError::Podman)?;

		let containers: Vec<ContainerListEntry> = all_containers
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
			// Full 64-char IDs, like `docker compose ps -q` (and podup's JSON),
			// so scripts consuming the IDs are not handed truncated values.
			for c in &containers {
				println!("{}", c.id);
			}
			return Ok(());
		}

		if opts.json {
			let rows: Vec<_> = containers.iter().map(ps_json_row).collect();
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
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::collections::HashMap;

	fn entry(status: &str, state: &str) -> ContainerListEntry {
		ContainerListEntry {
			id: "abc123".into(),
			names: vec!["/web".into()],
			image: "alpine".into(),
			status: status.into(),
			state: state.into(),
			ports: vec![],
			exit_code: None,
			labels: HashMap::new(),
		}
	}

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
	fn display_status_falls_back_to_state_when_status_empty() {
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
	fn format_ports_expands_a_collapsed_range() {
		// libpod collapses 51251-51253->8080-8082 into one record with range=3;
		// the full range must be rendered, not just the first mapping.
		let p = ContainerPort {
			host_ip: None,
			host_port: Some(51251),
			container_port: 8080,
			protocol: Some("tcp".into()),
			range: Some(3),
		};
		assert_eq!(
			format_ports(std::slice::from_ref(&p)),
			"0.0.0.0:51251-51253->8080-8082/tcp"
		);
	}

	#[test]
	fn publishers_expand_each_port_in_a_range() {
		let p = ContainerPort {
			host_ip: Some("0.0.0.0".into()),
			host_port: Some(51251),
			container_port: 8080,
			protocol: Some("tcp".into()),
			range: Some(3),
		};
		let pubs = publishers(std::slice::from_ref(&p));
		assert_eq!(pubs.len(), 3);
		assert_eq!(pubs[0]["TargetPort"], 8080);
		assert_eq!(pubs[0]["PublishedPort"], 51251);
		assert_eq!(pubs[2]["TargetPort"], 8082);
		assert_eq!(pubs[2]["PublishedPort"], 51253);
		assert_eq!(pubs[1]["Protocol"], "tcp");
	}

	#[test]
	fn health_is_derived_from_status_text() {
		assert_eq!(health_from_status("Up 2 minutes (healthy)"), "healthy");
		assert_eq!(health_from_status("Up 1 minute (unhealthy)"), "unhealthy");
		assert_eq!(
			health_from_status("Up 3 seconds (health: starting)"),
			"starting"
		);
		assert_eq!(health_from_status("Exited (1) 4 seconds ago"), "");
	}

	#[test]
	fn ps_json_row_surfaces_state_exitcode_and_publishers() {
		let mut labels = HashMap::new();
		labels.insert("podup.project".to_string(), "demo".to_string());
		labels.insert("podup.service".to_string(), "web".to_string());
		let c = ContainerListEntry {
			id: "deadbeef".into(),
			names: vec!["/demo-web-1".into()],
			image: "nginx:1.25".into(),
			status: "Exited (137) 2s ago".into(),
			state: "exited".into(),
			ports: vec![ContainerPort {
				host_ip: None,
				host_port: Some(8080),
				container_port: 80,
				protocol: Some("tcp".into()),
				range: None,
			}],
			exit_code: Some(137),
			labels,
		};
		let row = ps_json_row(&c);
		assert_eq!(row["Name"], "demo-web-1");
		assert_eq!(row["Service"], "web");
		assert_eq!(row["Project"], "demo");
		assert_eq!(row["State"], "exited");
		assert_eq!(row["ExitCode"], 137);
		assert_eq!(row["ID"], "deadbeef");
		assert_eq!(row["Publishers"][0]["PublishedPort"], 8080);
	}
}
