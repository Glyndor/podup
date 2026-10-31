use crate::compose::types::ComposeFile;
use crate::engine::lifecycle::targets::filter_services;
use crate::engine::Engine;
use crate::error::{ComposeError, Result};
use crate::libpod::API_PREFIX;

/// Display width of `wait`'s NAME column.
///
/// Fixed rather than content-sized: `wait` blocks and prints each container as
/// it exits, so there is no complete set of rows to measure. Wide enough for a
/// typical `<project>-<service>-<n>`; a longer name truncates the way every
/// other capped column in the binary does.
pub(super) const WAIT_NAME_WIDTH: usize = 32;

/// `wait`'s header row.
pub(super) fn wait_header() -> String {
	format!("{:<WAIT_NAME_WIDTH$} EXIT", "NAME")
}

/// One `wait` result line: the container, then its exit code coloured by whether
/// it is a failure.
///
/// The name goes through `fit_cell`, so it is escaped as well as padded: a
/// container name is not podup's own string.
pub(super) fn wait_row(container: &str, code: i64) -> String {
	let cell = crate::ui::fit_cell(container, WAIT_NAME_WIDTH);
	let style = if code == 0 {
		crate::ui::Style::new().dimmed()
	} else {
		crate::ui::Style::new().fg_color(Some(crate::ui::AnsiColor::Red.into()))
	};
	let coloured = crate::ui::stdout_colored();
	format!(
		"{} {}",
		crate::ui::paint(crate::ui::identity_style(container), &cell, coloured),
		crate::ui::paint(style, &code.to_string(), coloured)
	)
}

impl Engine {
	/// Block until each targeted service's containers stop, printing each exit
	/// code (`docker compose wait`). `json` emits one NDJSON object per
	/// container instead of the table.
	///
	/// Returns `RunExited` with the last non-zero code so the process exit
	/// status reflects it, mirroring docker compose.
	///
	/// NDJSON rather than a trailing array because `wait` blocks: a script
	/// that only learns anything once every container has exited has been
	/// handed the answer at the one moment it is least useful. It is also the
	/// rule `stats` already follows for its streaming output.
	pub async fn wait_services_with_options(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		json: bool,
	) -> Result<()> {
		// `docker compose wait` prints each service's exit code in the order the
		// services were given on the command line (deduplicated). Only fall back to
		// dependency order when no services were named (the "all" case).
		let order = if target_services.is_empty() {
			let order = crate::compose::resolve_order(file)?;
			filter_services(file, order, &[])?
		} else {
			for name in target_services {
				if !file.services.contains_key(name) {
					return Err(ComposeError::ServiceNotFound(name.clone()));
				}
			}
			let mut seen = std::collections::HashSet::new();
			target_services
				.iter()
				.filter(|n| seen.insert(n.as_str()))
				.cloned()
				.collect::<Vec<_>>()
		};

		// One line per container, which is the granularity the reference reports
		// at, measured against docker compose v5.1.3 with a service scaled to 3
		// replicas, it printed three lines, one per container id. The comment this
		// replaces asserted the opposite ("a single exit code for each service"),
		// and a bare `0` per service was built on it: with more than one container
		// nothing said which code belonged to which.
		//
		// What is deliberately *not* copied is the rendering. The reference prints
		// `container "<64-char hex id>" exited with status code 0`: an id nobody
		// can map back to a service, the same sentence repeated on every line, and
		// no machine path at all. Same information, said better: the container
		// name podup already has, aligned columns, and the code coloured by
		// whether it is a failure.
		let mut last_nonzero = 0i64;
		let mut printed_header = false;
		for name in &order {
			// Only wait on containers Podman actually has. The static-name fallback
			// would POST `/wait` to a never-created predicted name and surface a raw
			// HTTP 404; docker compose treats "nothing to wait on" as an idempotent
			// no-op, so a defined-but-never-created service is simply skipped (#758).
			for container_name in self
				.list_project_container_names(Some(name.as_str()))
				.await?
			{
				let path = format!(
					"{API_PREFIX}/containers/{}/wait?condition=stopped",
					crate::libpod::urlencoded(&container_name),
				);
				let code = self
					.client
					.post_empty_json_unbounded::<i64>(&path)
					.await
					.map_err(ComposeError::Podman)?;
				if code != 0 {
					last_nonzero = code;
				}
				if json {
					println!(
						"{}",
						serde_json::json!({ "Container": container_name, "ExitCode": code })
					);
				} else {
					// The header is printed lazily, with the first result: a project
					// where nothing was waited on stays the silent no-op it was,
					// rather than emitting a header over an empty table.
					if !printed_header {
						crate::ui::print_bold_header(&wait_header());
						printed_header = true;
					}
					println!("{}", wait_row(&container_name, code));
				}
			}
		}
		if last_nonzero != 0 {
			return Err(ComposeError::RunExited(last_nonzero));
		}
		Ok(())
	}
}
