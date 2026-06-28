//! `ls` — discover podup-managed compose projects on the host.
//!
//! Unlike the other commands this is project-agnostic: it scans every container
//! carrying a `podup.project` label and groups by project, so it needs only a
//! [`Client`], not a full [`Engine`](crate::engine::Engine) bound to one project/compose file.

use std::collections::BTreeMap;
use std::io::Write;

use crate::error::{ComposeError, Result};
use crate::libpod::types::container::ContainerListEntry;
use crate::libpod::{urlencoded, Client, API_PREFIX};

/// Options for [`list_projects`] (`docker compose ls`).
#[derive(Debug, Clone, Default)]
pub struct LsOptions {
	/// Include projects whose containers are all stopped.
	pub all: bool,
	/// Print only project names.
	pub quiet: bool,
	/// Emit a JSON array instead of a table.
	pub json: bool,
}

/// Whether a libpod `Status` string denotes a running container. Podman reports
/// `"running"` (or a human `"Up …"`) for live containers and `"exited"`/`"Exited
/// …"`/`"created"` otherwise. Pure so it can be unit-tested.
fn is_running(status: &str) -> bool {
	let s = status.trim();
	s.eq_ignore_ascii_case("running") || s.to_ascii_lowercase().starts_with("up")
}

/// A project's roll-up: running and total replica counts.
struct Tally {
	running: usize,
	total: usize,
}

/// List podup projects on the host (`docker compose ls`). Groups every
/// `podup.project`-labelled container by project; by default shows only
/// projects with at least one running container (`all` includes stopped ones).
pub async fn list_projects(client: &Client, opts: LsOptions) -> Result<()> {
	let filters = serde_json::json!({ "label": ["podup.project"] });
	let path = format!(
		"{API_PREFIX}/containers/json?all=true&filters={}",
		urlencoded(&filters.to_string()),
	);
	let containers = client
		.get_json::<Vec<ContainerListEntry>>(&path)
		.await
		.map_err(ComposeError::Podman)?;

	// Group by the project label, in name order for deterministic output.
	let mut projects: BTreeMap<String, Tally> = BTreeMap::new();
	for c in &containers {
		let Some(project) = c.labels.get("podup.project") else {
			continue;
		};
		let tally = projects.entry(project.clone()).or_insert(Tally {
			running: 0,
			total: 0,
		});
		tally.total += 1;
		// Podman's libpod list leaves `Status` empty and uses `State`; accept
		// either so the roll-up is robust across response shapes.
		if is_running(&c.state) || is_running(&c.status) {
			tally.running += 1;
		}
	}

	let rows: Vec<(&String, &Tally)> = projects
		.iter()
		.filter(|(_, t)| opts.all || t.running > 0)
		.collect();

	if opts.quiet {
		for (name, _) in &rows {
			println!("{name}");
		}
		return Ok(());
	}

	if opts.json {
		let arr: Vec<_> = rows
			.iter()
			.map(|(name, t)| serde_json::json!({ "Name": name, "Status": status_label(t) }))
			.collect();
		println!("{}", serde_json::to_string_pretty(&arr).unwrap_or_default());
		return Ok(());
	}

	let hdr = crate::ui::bold();
	let (on, off) = (hdr.render(), hdr.render_reset());
	let _ = writeln!(
		anstream::stdout(),
		"{on}{:<32} {:<20}{off}",
		"NAME",
		"STATUS"
	);
	for (name, t) in &rows {
		let status = crate::ui::status_cell(&status_label(t), 20);
		println!("{name:<32} {status}");
	}
	Ok(())
}

/// `running(N)` when any replica is up, else `exited(N)` — mirrors the
/// `docker compose ls` status column.
fn status_label(t: &Tally) -> String {
	if t.running > 0 {
		format!("running({})", t.running)
	} else {
		format!("exited({})", t.total)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn is_running_detects_live_statuses() {
		for up in ["running", "Up 2 minutes", "UP", "up about an hour"] {
			assert!(is_running(up), "{up} should be running");
		}
		for down in ["exited", "Exited (0) 3s ago", "created", "", "stopped"] {
			assert!(!is_running(down), "{down} should not be running");
		}
	}

	#[test]
	fn status_label_reflects_running_then_total() {
		assert_eq!(
			status_label(&Tally {
				running: 2,
				total: 3
			}),
			"running(2)"
		);
		assert_eq!(
			status_label(&Tally {
				running: 0,
				total: 3
			}),
			"exited(3)"
		);
	}
}
