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
}

/// Service/status/name filters for [`Engine::ps_filtered`] (`docker compose ps`
/// `--services`, `[SERVICE...]`, `--status`, `--filter`). Kept off the frozen
/// [`PsOptions`] struct so the published library API stays stable across minors.
#[derive(Default)]
pub struct PsFilterOptions {
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

/// Table STATUS cell. Podman's list endpoint reports an exited container with a
/// bare `exited` state and no code, which is indistinguishable from a clean exit;
/// surface the exit code the way `docker compose ps` does (`Exited (0)` /
/// `Exited (7)`). For every other state fall back to [`display_status`].
fn table_status(c: &ContainerListEntry) -> String {
	let status = display_status(c);
	let exited = c.state.eq_ignore_ascii_case("exited") || c.state.eq_ignore_ascii_case("dead");
	// Only synthesize when the status text doesn't already carry the code, so a
	// richer Docker-style `Exited (7) 4 seconds ago` is left untouched.
	if exited && !status.contains("Exited (") {
		return format!("Exited ({})", c.exit_code.unwrap_or(0));
	}
	status.to_string()
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
	} else if s.contains("health: starting") {
		"starting"
	} else {
		""
	}
}

/// Number of consecutive ports a record covers (`range`, at least 1).
fn span_len(p: &ContainerPort) -> u16 {
	p.range.filter(|&r| r > 0).unwrap_or(1)
}

/// `base + offset` widened to `u32` before adding. `host_port`,
/// `container_port` and `range` all come straight off libpod's JSON — untrusted
/// input a hostile or buggy daemon could set to any `u16` value — so a plain
/// `u16 + u16` (e.g. `host_port: 65535` with a `range` of 2) can overflow: it
/// wraps silently in a release build and panics under overflow-checks. Doing
/// the addition in `u32` keeps every legitimate port value identical while a
/// pathological one renders as the (larger) real number instead of wrapping.
fn widen_add(base: u16, offset: u16) -> u32 {
	u32::from(base) + u32::from(offset)
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
			widen_add(hp, n - 1),
			p.container_port,
			widen_add(p.container_port, n - 1),
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
				"TargetPort": widen_add(p.container_port, i),
				"PublishedPort": p.host_port.map(|hp| widen_add(hp, i)),
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

	/// List containers with `docker compose ps`-style options (`-a/--all`,
	/// `-q/--quiet`, `--format`). For the `--services`/`[SERVICE...]`/`--status`/
	/// `--filter` predicates use [`Engine::ps_filtered`].
	pub async fn ps_with_options(&self, file: &ComposeFile, opts: PsOptions) -> Result<()> {
		self.ps_filtered(file, opts, PsFilterOptions::default())
			.await
	}

	/// List containers with `docker compose ps`-style options: `-a/--all`
	/// (include stopped), `-q/--quiet` (full IDs only), `--format`
	/// (table | json), `--services` (service-name list), a positional `SERVICE`
	/// filter, and `--status`/`--filter` predicates.
	pub async fn ps_filtered(
		&self,
		file: &ComposeFile,
		opts: PsOptions,
		filters: PsFilterOptions,
	) -> Result<()> {
		for name in &filters.services {
			if !file.services.contains_key(name) {
				return Err(ComposeError::ServiceNotFound(name.clone()));
			}
		}

		// `--services` lists the (optionally filtered) configured service names,
		// one per line, instead of the container table.
		if filters.services_only {
			for name in file.services.keys() {
				if filters.services.is_empty() || filters.services.iter().any(|s| s == name) {
					println!("{name}");
				}
			}
			return Ok(());
		}

		// Fold `--status` and any `status=`/`name=` from `--filter` together. An
		// unsupported key is an error, not a warning: a dropped predicate means
		// the command answers a question the caller did not ask, and a script
		// filtering for a condition reads the unfiltered set back as a match.
		// docker compose errors here too.
		let (mut status_filter, name_filter, unknown) = split_ps_filters(&filters.filters);
		if let Some(u) = unknown.first() {
			return Err(ComposeError::Unsupported(format!(
				"unsupported ps filter {u:?}: expected name=<NAME> or status=<STATE>"
			)));
		}
		status_filter.extend(filters.status.iter().cloned());

		// A positional `SERVICE` filter restricts to those services' container
		// names (across replicas).
		let allowed_names: Option<std::collections::HashSet<String>> =
			if filters.services.is_empty() {
				None
			} else {
				Some(
					filters
						.services
						.iter()
						.filter_map(|n| file.services.get(n).map(|s| (n, s)))
						.flat_map(|(n, s)| self.replica_names(n, s))
						.collect(),
				)
			};

		// A status filter (`--status exited`, `--filter status=exited`) implies
		// querying every container regardless of state: libpod's list endpoint
		// with `all=false` returns only running containers, so `ps --status
		// exited` without `-a` would always come back empty. `status_matches`
		// below still narrows the result to the requested status(es); this only
		// widens what libpod itself is asked for.
		let all = opts.all || !status_filter.is_empty();
		let label = format!("podup.project={}", self.project);
		let filters = serde_json::json!({ "label": [label] });
		let path = format!(
			"{API_PREFIX}/containers/json?all={}&filters={}",
			all,
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

		let mut table = crate::ui::Table::new(&["NAME", "IMAGE", "STATUS", "PORTS"])
			.cap(0, 48)
			.cap(1, 48)
			.status_col(2)
			.identity_col(0);
		for c in &containers {
			table.push(vec![
				name_of(c),
				c.image.clone(),
				table_status(c),
				format_ports(&c.ports),
			]);
		}
		table.print();

		Ok(())
	}
}

#[cfg(test)]
mod tests;
