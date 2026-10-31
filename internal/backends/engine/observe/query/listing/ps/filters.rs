//! The `--status` and `--filter` predicates for `ps`. Pure so each rule can be
//! unit-tested without a live Podman socket, and so the same code answers the
//! JSON path's filter pass and the table path's filter pass.

use std::collections::HashSet;

use crate::libpod::types::container::ContainerListEntry;

use super::format::name_of;

/// The service filter: the services a positional `SERVICE` argument selected,
/// and the container names the compose file predicts for them.
pub(super) struct ServiceFilter {
	/// The selected service names, as the caller spelled them.
	services: HashSet<String>,
	/// The container names the compose file predicts for those services.
	names: HashSet<String>,
}

impl ServiceFilter {
	/// Build the filter for the services a positional argument named.
	pub(super) fn new(services: HashSet<String>, names: HashSet<String>) -> Self {
		Self { services, names }
	}

	/// Whether a container belongs to one of the selected services.
	///
	/// The `podup.service` label decides it: it names the service that owns
	/// the container, and the daemon reports it for every container the
	/// project owns. The container names cannot decide it on their own,
	/// because they come from the compose file's replica count: a service
	/// scaled at runtime (`up --scale web=3`) has three containers and one
	/// name in the file, so the two extra replicas were dropped from `ps web`
	/// even though the daemon had them right there in the listing.
	///
	/// A container with no service label has no other evidence to offer, so
	/// its name is matched against the names the compose file predicts.
	pub(super) fn keeps(&self, c: &ContainerListEntry) -> bool {
		match c.labels.get("podup.service") {
			Some(service) => self.services.contains(service),
			None => c
				.names
				.iter()
				.any(|n| self.names.contains(n.trim_start_matches('/'))),
		}
	}
}

/// The containers a `ps` invocation shows: the project's own, minus the infra
/// container Podman creates inside the project pod, narrowed by the service,
/// status and name predicates.
///
/// One function for the table path and the test-only row helper, so the rows
/// a test inspects are the rows the command prints.
pub(super) fn select_containers(
	containers: Vec<ContainerListEntry>,
	service: Option<&ServiceFilter>,
	status: &[String],
	name: &[String],
) -> Vec<ContainerListEntry> {
	containers
		.into_iter()
		.filter(|c| {
			// `x-podman-pod`: hide the infra container Podman creates
			// inside the project pod. The pod addresses itself by name
			// (`podman pod`), so the infra container has no user-facing
			// role.
			if c.is_infra {
				return false;
			}
			let display = name_of(c);
			service.is_none_or(|s| s.keeps(c))
				&& (status_matches(&c.state, status) || status_matches(&c.status, status))
				&& (name.is_empty() || name.iter().any(|nf| display.contains(nf)))
		})
		.collect()
}

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
