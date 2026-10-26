//! `ps` lists this project's containers as a table or JSON. Split into one
//! file per responsibility: options, the cell renderers, the port formatters,
//! the filter predicates, the JSON row builder, the Engine entry points and
//! the test-only projection helper.

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::types::container::ContainerListEntry;

use super::Engine;

mod filters;
mod format;
mod json;
mod options;
mod ports;

pub use options::{PsDisplayOptions, PsFilterOptions, PsOptions};

// `now_unix` is read by `crate::engine::query::images` too; this reach keeps both
// halves on the same clock without widening it to the whole crate.
pub(in crate::backends::engine) use format::now_unix;

use filters::{split_ps_filters, status_matches};
use format::{containers_path, name_of, table_created, table_service, table_size, table_status};
use json::ps_json_row;
use ports::format_ports;

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
		self.ps_filtered_with_display(file, opts, filters, PsDisplayOptions::default())
			.await
	}

	/// Like [`Engine::ps_filtered`], plus the options added after the API froze
	/// ([`PsDisplayOptions`]). A separate entry point rather than a fourth
	/// parameter on the old one, so existing callers keep compiling.
	pub async fn ps_filtered_with_display(
		&self,
		file: &ComposeFile,
		opts: PsOptions,
		filters: PsFilterOptions,
		display: PsDisplayOptions,
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
		let path = containers_path(&self.project, all, display.size);

		let all_containers = self
			.client
			.get_json::<Vec<ContainerListEntry>>(&path)
			.await
			.map_err(ComposeError::Podman)?;

		let containers: Vec<ContainerListEntry> = all_containers
			.into_iter()
			.filter(|c| {
				// `x-podman-pod`: hide the infra container Podman creates
				// inside the project pod. The pod addresses itself by name
				// (`podman pod`), so the infra container has no user-facing
				// role.
				if c.is_infra {
					return false;
				}
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
			println!("{}", crate::engine::to_pretty_json("ps.row", &rows)?);
			return Ok(());
		}

		// One clock read for the whole table, so two rows created in the same
		// second cannot render different ages.
		let now = now_unix();
		// `SERVICE` sits after IMAGE and before the timestamps, in the same slot
		// `docker compose ps` puts it. The status_col/identity_col indices below
		// assume this layout.
		let mut headers: Vec<&str> = vec!["NAME", "IMAGE", "SERVICE", "CREATED", "STATUS", "PORTS"];
		if display.size {
			headers.push("SIZE");
		}
		let mut table = crate::ui::Table::new(&headers)
			.cap(0, 48)
			.cap(1, 48)
			.status_col(4)
			.identity_col(0);
		for c in &containers {
			let mut row = vec![
				name_of(c),
				c.image.clone(),
				table_service(c),
				table_created(c, now),
				table_status(c, now),
				format_ports(&c.ports),
			];
			if display.size {
				row.push(table_size(c));
			}
			table.push(row);
		}
		if table.is_empty() {
			// An empty ps table is a legitimate answer; print the explicit
			// "no containers" line on stderr so a script capturing stdout (or
			// `--format json`) sees nothing (#1675).
			crate::ui::progress_note("no containers");
			return Ok(());
		}
		table.print();

		Ok(())
	}
}

/// Test-only: returns the JSON row map a `ps` invocation would render, so
/// the unit tests can introspect the filtered container set without going
/// through stdout.
#[cfg(test)]
pub(crate) async fn ps_rows_for_test(
	engine: &Engine,
	file: &ComposeFile,
) -> Result<Vec<serde_json::Value>> {
	let opts = PsOptions::new(true, false, true);
	let filters = PsFilterOptions::default();
	let display = PsDisplayOptions::default();
	// Re-run the same filter the production path runs, but capture the
	// resulting rows instead of printing them.
	let path = containers_path(&engine.project, opts.all || true, display.size);
	let all_containers = engine
		.client
		.get_json::<Vec<ContainerListEntry>>(&path)
		.await
		.map_err(ComposeError::Podman)?;
	let allowed_names: Option<std::collections::HashSet<String>> = if filters.services.is_empty() {
		None
	} else {
		Some(
			filters
				.services
				.iter()
				.filter_map(|n| file.services.get(n).map(|s| (n, s)))
				.flat_map(|(n, s)| engine.replica_names(n, s))
				.collect(),
		)
	};
	let (status_filter, name_filter, _) = split_ps_filters(&filters.filters);
	let containers: Vec<ContainerListEntry> = all_containers
		.into_iter()
		.filter(|c| {
			if c.is_infra {
				return false;
			}
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
	Ok(containers.iter().map(ps_json_row).collect())
}

#[cfg(test)]
mod tests;
