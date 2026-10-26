//! The `--status` and `--filter` predicates for `ps`. Pure so each rule can be
//! unit-tested without a live Podman socket, and so the same code answers the
//! JSON path's filter pass and the table path's filter pass.

/// Whether a container status/state word satisfies a `--status`/`status=` filter.
/// Each wanted value matches case-insensitively as a prefix of the status word
/// (so `running` matches `running` and `up`-style strings via the state). An
/// empty `wanted` matches everything. Pure so the predicate is unit-tested.
pub(super) fn status_matches(status: &str, wanted: &[String]) -> bool {
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
pub(super) fn split_ps_filters(filters: &[String]) -> (Vec<String>, Vec<String>, Vec<String>) {
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
pub(super) fn health_from_status(status: &str) -> &'static str {
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
