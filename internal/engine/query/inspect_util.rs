//! Pure helpers for `inspect`: replica/port selection, image-ref
//! splitting, and process-table formatting. Kept free of the live Podman
//! client so each is unit-tested in isolation.

use crate::error::{ComposeError, Result};

/// Pick a service's target replica container from its live container names.
///
/// Names are ordered by their trailing `-N` suffix (numerically, so `svc-10`
/// sorts after `svc-2`); an unsuffixed single-replica name sorts first. `index`
/// is the 1-based `--index`; `None` selects the first replica. Pure so the
/// indexing is unit-tested without a live Podman socket.
pub(super) fn select_replica(
	mut names: Vec<String>,
	service_name: &str,
	index: Option<u32>,
) -> Result<String> {
	names.sort_by_key(|n| {
		n.rsplit_once('-')
			.and_then(|(_, suffix)| suffix.parse::<u64>().ok())
			.unwrap_or(0)
	});
	match index {
		Some(i) => {
			// `--index` is 1-based; `0` is invalid, not "first replica".
			let idx = (i as usize).checked_sub(1).ok_or_else(|| {
				ComposeError::ServiceNotFound(format!(
					"{service_name} (replica index {i}: indexes are 1-based)"
				))
			})?;
			names.get(idx).cloned().ok_or_else(|| {
				ComposeError::ServiceNotFound(format!("{service_name} (replica index {i})"))
			})
		}
		None => names
			.into_iter()
			.next()
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into())),
	}
}

/// Resolve the `(port, proto)` for `port` from a `PORT` or `PORT/proto` argument,
/// the `/proto` suffix overriding the `--protocol` flag — matching
/// `docker compose port`. Pure so the parsing is unit-tested.
pub(super) fn parse_port_proto<'a>(
	private_port: &'a str,
	proto_flag: &'a str,
) -> Result<(u16, &'a str)> {
	let (port, proto) = match private_port.split_once('/') {
		Some((p, pr)) => (p, pr),
		None => (private_port, proto_flag),
	};
	let port: u16 = port.parse().map_err(|_| {
		ComposeError::InvalidPort(format!(
			"port '{private_port}' is not a valid PORT or PORT/proto"
		))
	})?;
	Ok((port, proto))
}

/// Deduplicate a list of strings, preserving first-seen order. Used so `top web
/// web` queries and prints each service once, matching `docker compose top`.
pub(super) fn dedup_preserving_order(items: &[String]) -> Vec<String> {
	let mut seen = std::collections::HashSet::new();
	items
		.iter()
		.filter(|s| seen.insert(s.as_str()))
		.cloned()
		.collect()
}

/// Whether a libpod container `Status` string denotes a running container.
/// `docker compose attach` only attaches to a running container; anything else
/// (exited, created, paused, empty/unknown) must fail closed.
pub(super) fn is_running_status(status: &str) -> bool {
	status.eq_ignore_ascii_case("running")
}

/// Split an image reference into `(repository, tag)` for the `images` table.
///
/// A trailing `:tag` is only a tag when the segment after it has no `/` (so a
/// `registry:port/name` host is not mis-split), mirroring the guard in
/// `export.rs`. A `name@sha256:...` digest reference has no tag, shown as
/// `<none>` like docker, and the long digest never bloats the TAG column.
pub(super) fn split_repo_tag(image_ref: &str) -> (String, String) {
	if let Some((repo, _digest)) = image_ref.split_once('@') {
		return (repo.to_string(), "<none>".to_string());
	}
	match image_ref.rsplit_once(':') {
		Some((repo, tag)) if !tag.contains('/') => (repo.to_string(), tag.to_string()),
		_ => (image_ref.to_string(), "latest".to_string()),
	}
}

/// Align a `top` table (the title row followed by process rows) into
/// space-padded columns sized to the widest cell, returning one rendered line
/// per input row (titles first). All but the last column are left-padded; the
/// last is left ragged to avoid trailing whitespace.
pub(super) fn align_top_columns(titles: &[String], processes: &[Vec<String>]) -> Vec<String> {
	let mut rows: Vec<&[String]> = Vec::with_capacity(processes.len() + 1);
	if !titles.is_empty() {
		rows.push(titles);
	}
	for p in processes {
		rows.push(p);
	}
	let col_count = rows.iter().map(|r| r.len()).max().unwrap_or(0);
	let mut widths = vec![0usize; col_count];
	for r in &rows {
		for (i, cell) in r.iter().enumerate() {
			widths[i] = widths[i].max(cell.chars().count());
		}
	}
	rows.iter()
		.map(|r| {
			r.iter()
				.enumerate()
				.map(|(i, cell)| {
					if i + 1 == r.len() {
						cell.clone()
					} else {
						format!("{cell:<width$}", width = widths[i])
					}
				})
				.collect::<Vec<_>>()
				.join("  ")
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::{
		align_top_columns, dedup_preserving_order, is_running_status, parse_port_proto,
		select_replica, split_repo_tag,
	};

	#[test]
	fn select_replica_none_picks_first_by_suffix() {
		// Live names come back in arbitrary API order; the first replica is the
		// lowest-suffixed one regardless.
		let names = vec![
			"proj-web-3".into(),
			"proj-web-1".into(),
			"proj-web-2".into(),
		];
		assert_eq!(select_replica(names, "web", None).unwrap(), "proj-web-1");
	}

	#[test]
	fn select_replica_orders_suffix_numerically() {
		// `-10` must sort after `-2`, not lexicographically before it.
		let names = vec![
			"proj-web-10".into(),
			"proj-web-2".into(),
			"proj-web-1".into(),
		];
		assert_eq!(
			select_replica(names, "web", Some(3)).unwrap(),
			"proj-web-10"
		);
	}

	#[test]
	fn select_replica_index_targets_nth() {
		let names = vec!["proj-web-1".into(), "proj-web-2".into()];
		assert_eq!(
			select_replica(names.clone(), "web", Some(2)).unwrap(),
			"proj-web-2"
		);
	}

	#[test]
	fn select_replica_unsuffixed_single() {
		let names = vec!["proj-web".into()];
		assert_eq!(select_replica(names, "web", None).unwrap(), "proj-web");
	}

	#[test]
	fn select_replica_rejects_index_zero_and_out_of_range() {
		let names = vec!["proj-web-1".into(), "proj-web-2".into()];
		assert!(select_replica(names.clone(), "web", Some(0)).is_err());
		assert!(select_replica(names, "web", Some(5)).is_err());
	}

	#[test]
	fn select_replica_empty_is_not_found() {
		assert!(select_replica(vec![], "web", None).is_err());
	}

	#[test]
	fn split_repo_tag_plain_name_and_tag() {
		assert_eq!(
			split_repo_tag("nginx:1.25"),
			("nginx".into(), "1.25".into())
		);
		assert_eq!(split_repo_tag("nginx"), ("nginx".into(), "latest".into()));
	}

	#[test]
	fn split_repo_tag_registry_with_port_is_not_a_tag() {
		// The ':' belongs to the registry host:port, not a tag.
		assert_eq!(
			split_repo_tag("registry:5000/team/app"),
			("registry:5000/team/app".into(), "latest".into())
		);
		assert_eq!(
			split_repo_tag("registry:5000/team/app:v2"),
			("registry:5000/team/app".into(), "v2".into())
		);
	}

	#[test]
	fn split_repo_tag_digest_has_no_tag() {
		let (repo, tag) = split_repo_tag(
			"docker.io/library/alpine@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
		);
		assert_eq!(repo, "docker.io/library/alpine");
		assert_eq!(tag, "<none>");
	}

	#[test]
	fn dedup_preserving_order_keeps_first_occurrence() {
		let out =
			dedup_preserving_order(&["web".into(), "db".into(), "web".into(), "cache".into()]);
		assert_eq!(out, vec!["web", "db", "cache"]);
	}

	#[test]
	fn align_top_columns_pads_to_widest_cell() {
		let titles = vec!["PID".to_string(), "CMD".to_string()];
		let processes = vec![
			vec!["1".to_string(), "bash".to_string()],
			vec!["12345".to_string(), "node".to_string()],
		];
		let lines = align_top_columns(&titles, &processes);
		assert_eq!(lines.len(), 3);
		// First column is padded to the widest value ("12345" = 5 chars).
		assert!(lines[0].starts_with("PID  "));
		assert!(lines[1].starts_with("1      "));
		// No tabs in the aligned output.
		assert!(lines.iter().all(|l| !l.contains('\t')));
	}

	#[test]
	fn bare_port_uses_flag_proto() {
		assert_eq!(parse_port_proto("80", "tcp").unwrap(), (80, "tcp"));
	}

	#[test]
	fn suffix_overrides_flag_proto() {
		assert_eq!(parse_port_proto("53/udp", "tcp").unwrap(), (53, "udp"));
	}

	#[test]
	fn non_numeric_port_is_rejected() {
		assert!(parse_port_proto("http", "tcp").is_err());
		assert!(parse_port_proto("abc/tcp", "tcp").is_err());
	}

	#[test]
	fn dedup_keeps_first_occurrence_order() {
		let input = ["web".to_string(), "web".to_string(), "db".to_string()];
		assert_eq!(dedup_preserving_order(&input), vec!["web", "db"]);
		let input = [
			"a".to_string(),
			"b".to_string(),
			"a".to_string(),
			"c".to_string(),
			"b".to_string(),
		];
		assert_eq!(dedup_preserving_order(&input), vec!["a", "b", "c"]);
	}

	#[test]
	fn running_status_detected_case_insensitively() {
		assert!(is_running_status("running"));
		assert!(is_running_status("Running"));
		// Anything else is not attachable.
		assert!(!is_running_status("exited"));
		assert!(!is_running_status("created"));
		assert!(!is_running_status("paused"));
		assert!(!is_running_status(""));
	}
}
