//! `ls` — discover podup-managed compose projects on the host.
//!
//! Unlike the other commands this is project-agnostic: it scans every container
//! carrying a `podup.project` label and groups by project, so it needs only a
//! [`Client`], not a full [`Engine`](crate::engine::Engine) bound to one project/compose file.

use std::collections::BTreeMap;

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

/// Split `ls --filter KEY=VALUE` predicates into name, status, and unknown
/// buckets. Pure so it is unit-tested.
fn split_ls_filters(filters: &[String]) -> (Vec<String>, Vec<String>, Vec<String>) {
	let (mut names, mut status, mut unknown) = (Vec::new(), Vec::new(), Vec::new());
	for f in filters {
		match f.split_once('=') {
			Some(("name", v)) => names.push(v.to_string()),
			Some(("status", v)) => status.push(v.to_ascii_lowercase()),
			_ => unknown.push(f.clone()),
		}
	}
	(names, status, unknown)
}

/// Whether a project row passes the parsed name/status filters. `running` is the
/// project's roll-up running flag. Pure so it is unit-tested.
fn ls_row_matches(name: &str, running: bool, names: &[String], status: &[String]) -> bool {
	let name_ok = names.is_empty() || names.iter().any(|n| name.contains(n.as_str()));
	let status_word = if running { "running" } else { "exited" };
	let status_ok = status.is_empty() || status.iter().any(|s| s == status_word);
	name_ok && status_ok
}

/// Whether a libpod `Status` string denotes a running container. Podman reports
/// `"running"` (or a human `"Up …"`) for live containers and `"exited"`/`"Exited
/// …"`/`"created"` otherwise. Pure so it can be unit-tested.
fn is_running(status: &str) -> bool {
	let s = status.trim();
	s.eq_ignore_ascii_case("running") || s.to_ascii_lowercase().starts_with("up")
}

/// Whether a libpod `Status`/`State` string denotes a paused container. Podman
/// reports `"paused"` for the machine state and `"Paused"` in the human status.
/// Pure so it can be unit-tested. `docker compose ls` surfaces this state rather
/// than hiding the project or mislabelling it as exited.
fn is_paused(status: &str) -> bool {
	status.trim().to_ascii_lowercase().starts_with("paus")
}

/// A project's roll-up: running, paused, and total replica counts. Stopped
/// replicas are the remainder (`total - running - paused`).
struct Tally {
	running: usize,
	paused: usize,
	total: usize,
}

/// List podup projects on the host (`docker compose ls`). Groups every
/// `podup.project`-labelled container by project; by default shows only
/// projects with at least one running container (`all` includes stopped ones).
/// For the `--filter name=/status=` predicates use [`list_projects_filtered`].
pub async fn list_projects(client: &Client, opts: LsOptions) -> Result<()> {
	list_projects_filtered(client, opts, &[]).await
}

/// List podup projects (`docker compose ls`) narrowed by `--filter` predicates
/// (`name=<NAME>`, `status=<running|exited>`). The `filters` slice is kept off
/// the frozen [`LsOptions`] struct so the 1.0 library API stays stable.
pub async fn list_projects_filtered(
	client: &Client,
	opts: LsOptions,
	filters: &[String],
) -> Result<()> {
	let label_filters = serde_json::json!({ "label": ["podup.project"] });
	let path = format!(
		"{API_PREFIX}/containers/json?all=true&filters={}",
		urlencoded(&label_filters.to_string()),
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
			paused: 0,
			total: 0,
		});
		tally.total += 1;
		// Podman's libpod list leaves `Status` empty and uses `State`; accept
		// either so the roll-up is robust across response shapes. A paused
		// container is counted separately so it is neither hidden nor mislabelled
		// as exited.
		if is_running(&c.state) || is_running(&c.status) {
			tally.running += 1;
		} else if is_paused(&c.state) || is_paused(&c.status) {
			tally.paused += 1;
		}
	}

	let (name_filter, status_filter, unknown) = split_ls_filters(filters);
	for u in &unknown {
		tracing::warn!("ls: ignoring unsupported filter '{u}'");
	}
	// A project is "active" (shown without `--all`) when any replica is running
	// or paused; only all-stopped projects are hidden by default. The `--filter`
	// name=/status= predicates further narrow the shown rows.
	let rows: Vec<(&String, &Tally)> = projects
		.iter()
		.filter(|(_, t)| opts.all || t.running > 0 || t.paused > 0)
		.filter(|(name, t)| ls_row_matches(name, t.running > 0, &name_filter, &status_filter))
		.collect();

	if opts.quiet {
		for (name, _) in &rows {
			println!("{name}");
		}
		return Ok(());
	}

	if opts.json {
		let arr: Vec<_> = rows.iter().map(|(name, t)| project_row(name, t)).collect();
		println!("{}", serde_json::to_string_pretty(&arr).unwrap_or_default());
		return Ok(());
	}

	let mut table = crate::ui::Table::new(&["NAME", "STATUS"])
		.cap(0, 48)
		.status_col(1);
	for (name, t) in &rows {
		table.push(vec![name.to_string(), status_label(t)]);
	}
	table.print();
	Ok(())
}

/// Per-state replica counts joined as `running(2), paused(1), exited(1)` —
/// mirrors the `docker compose ls` status column, which surfaces each state
/// rather than collapsing to a single running count and discarding the rest.
fn status_label(t: &Tally) -> String {
	let exited = t.total.saturating_sub(t.running).saturating_sub(t.paused);
	let mut parts = Vec::new();
	if t.running > 0 {
		parts.push(format!("running({})", t.running));
	}
	if t.paused > 0 {
		parts.push(format!("paused({})", t.paused));
	}
	if exited > 0 {
		parts.push(format!("exited({exited})"));
	}
	if parts.is_empty() {
		// No replicas at all (an edge case); report a zero exited count.
		parts.push(format!("exited({})", t.total));
	}
	parts.join(", ")
}

/// One `ls --format json` row. `ConfigFiles` is always present for parity with
/// `docker compose ls --format json`; podup discovers projects by container
/// label and tracks no compose path per project, so the field is empty.
fn project_row(name: &str, t: &Tally) -> serde_json::Value {
	serde_json::json!({
		"Name": name,
		"Status": status_label(t),
		"ConfigFiles": "",
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn is_running_detects_live_statuses() {
		for up in ["running", "Up 2 minutes", "UP", "up about an hour"] {
			assert!(is_running(up), "{up} should be running");
		}
		for down in [
			"exited",
			"Exited (0) 3s ago",
			"created",
			"",
			"stopped",
			"paused",
		] {
			assert!(!is_running(down), "{down} should not be running");
		}
	}

	#[test]
	fn split_ls_filters_buckets_and_flags_unknown() {
		let (names, status, unknown) = split_ls_filters(&[
			"name=web".to_string(),
			"status=RUNNING".to_string(),
			"bogus=1".to_string(),
		]);
		assert_eq!(names, vec!["web".to_string()]);
		assert_eq!(status, vec!["running".to_string()]);
		assert_eq!(unknown, vec!["bogus=1".to_string()]);
	}

	#[test]
	fn ls_row_matches_applies_name_and_status() {
		// No filters → always matches.
		assert!(ls_row_matches("app", true, &[], &[]));
		// name substring.
		assert!(ls_row_matches("myapp", true, &["app".to_string()], &[]));
		assert!(!ls_row_matches("other", true, &["app".to_string()], &[]));
		// status word.
		assert!(ls_row_matches("app", true, &[], &["running".to_string()]));
		assert!(!ls_row_matches("app", false, &[], &["running".to_string()]));
		assert!(ls_row_matches("app", false, &[], &["exited".to_string()]));
	}

	#[test]
	fn is_paused_detects_paused_statuses() {
		for p in ["paused", "Paused", "PAUSED"] {
			assert!(is_paused(p), "{p} should be paused");
		}
		for other in ["running", "exited", "created", ""] {
			assert!(!is_paused(other), "{other} should not be paused");
		}
	}

	#[test]
	fn status_label_emits_per_state_counts() {
		// Mixed running + stopped keeps both counts instead of dropping the down one.
		assert_eq!(
			status_label(&Tally {
				running: 2,
				paused: 0,
				total: 3
			}),
			"running(2), exited(1)"
		);
		// A paused project is labelled paused, not exited.
		assert_eq!(
			status_label(&Tally {
				running: 0,
				paused: 1,
				total: 1
			}),
			"paused(1)"
		);
		// All up, all states present.
		assert_eq!(
			status_label(&Tally {
				running: 1,
				paused: 1,
				total: 3
			}),
			"running(1), paused(1), exited(1)"
		);
		assert_eq!(
			status_label(&Tally {
				running: 0,
				paused: 0,
				total: 3
			}),
			"exited(3)"
		);
	}

	#[test]
	fn project_row_includes_config_files_field() {
		let row = project_row(
			"web",
			&Tally {
				running: 1,
				paused: 0,
				total: 1,
			},
		);
		assert_eq!(row["Name"], "web");
		assert_eq!(row["Status"], "running(1)");
		// Present for docker-compose parity even though podup tracks no path.
		assert_eq!(row["ConfigFiles"], "");
	}
}
