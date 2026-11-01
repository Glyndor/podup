use futures_util::stream::FuturesUnordered;
use futures_util::StreamExt;

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
	/// Returns the last completed container's code, including zero, matching
	/// Docker Compose v2.40.3. Wait failures remain errors after all waits finish.
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
		// Validate and deduplicate targets before starting any waits.
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

		let mut waits = Vec::new();
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
				waits.push(async move {
					let code = self
						.client
						.post_empty_json_unbounded::<i64>(&path)
						.await
						.map_err(ComposeError::Podman);
					(container_name, code)
				});
			}
		}
		let mut printed_header = false;
		wait_for_containers(waits, |container_name, code| {
			if json {
				println!(
					"{}",
					serde_json::json!({ "Container": container_name, "ExitCode": code })
				);
			} else {
				if !printed_header {
					crate::ui::print_bold_header(&wait_header());
					printed_header = true;
				}
				println!("{}", wait_row(container_name, code));
			}
		})
		.await
	}
}

async fn wait_for_containers<F>(
	waits: impl IntoIterator<Item = F>,
	mut report: impl FnMut(&str, i64),
) -> Result<()>
where
	F: std::future::Future<Output = (String, Result<i64>)>,
{
	let mut pending: FuturesUnordered<_> = waits.into_iter().collect();
	let mut last_code = 0;
	let mut first_error = None;
	while let Some((container, result)) = pending.next().await {
		match result {
			Ok(code) => {
				last_code = code;
				report(&container, code);
			}
			Err(error) => {
				first_error.get_or_insert(error);
			}
		}
	}
	if let Some(error) = first_error {
		return Err(error);
	}
	if last_code != 0 {
		return Err(ComposeError::RunExited(last_code));
	}
	Ok(())
}

#[cfg(test)]
#[path = "wait_tests.rs"]
mod tests;
