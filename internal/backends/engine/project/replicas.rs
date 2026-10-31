//! Replica-name resolution and ordering helpers.
//!
//! Pulled out of `engine::mod` so the engine module stays under the 500-line
//! hard cap enforced by the org's `line-limit` reusable. The helpers cluster
//! around one concept and stay together: `resolve_replica_name` for `--index`
//! lookup, `order_replicas` for the "lowest-numbered replica" rule, and the
//! `Engine` methods that derive replica sets from a [`Service`]. A caller
//! that needs one of them reads the others without having to chase their
//! declarations across modules.

use crate::compose::types::Service;
use crate::error::{ComposeError, Result};

use crate::engine::Engine;

/// Resolve a replica container name from the set of names that exist for a
/// service (the running replicas, or the statically derived names before
/// anything is created) and a 1-based `--index`. Each name is either the
/// unsuffixed base (the sole replica) or `{base}-{n}`.
///
/// `--index n` targets the replica numbered `n` by name, not by position,
/// so it stays correct after a runtime `scale`/`up --scale` and regardless of
/// the order Podman lists containers; `0` is rejected (indexes are 1-based);
/// `None` picks the lowest-numbered replica. Pure so it is unit-testable
/// without a Podman socket.
pub(in crate::backends::engine) fn resolve_replica_name(
	service_name: &str,
	base: &str,
	names: &[String],
	index: Option<u32>,
) -> Result<String> {
	match index {
		Some(0) => Err(ComposeError::ReplicaIndex {
			service: service_name.to_string(),
			index: 0,
		}),
		Some(i) => {
			let suffixed = format!("{base}-{i}");
			if names.iter().any(|n| n == &suffixed) {
				return Ok(suffixed);
			}
			// A single, unsuffixed replica answers to index 1 only.
			if i == 1 && names.iter().any(|n| n == base) {
				return Ok(base.to_string());
			}
			Err(ComposeError::ReplicaIndex {
				service: service_name.to_string(),
				index: i,
			})
		}
		None => order_replicas(base, names)
			.into_iter()
			.next()
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into())),
	}
}

/// Order replica container names by their 1-based replica number so callers can
/// pick the lowest-numbered one independently of Podman's listing order. A name
/// is the unsuffixed base (the sole replica → number 1) or `{base}-{n}`; names
/// matching neither are dropped.
pub(in crate::backends::engine) fn order_replicas(base: &str, names: &[String]) -> Vec<String> {
	let prefix = format!("{base}-");
	let mut numbered: Vec<(usize, String)> = names
		.iter()
		.filter_map(|name| {
			if name == base {
				Some((1, name.clone()))
			} else {
				name.strip_prefix(&prefix)
					.and_then(|s| s.parse::<usize>().ok())
					.map(|n| (n, name.clone()))
			}
		})
		.collect();
	numbered.sort_by_key(|(n, _)| *n);
	numbered.into_iter().map(|(_, name)| name).collect()
}

impl Engine {
	/// Resolve the replica count for a service: a CLI `--scale` override wins,
	/// else the compose `scale:`, else `deploy.replicas`, else 1. The single
	/// source of truth so `up`, naming, and teardown never drift.
	pub(in crate::backends::engine) fn resolve_replicas(
		&self,
		service_name: &str,
		service: &Service,
	) -> usize {
		if let Some(&n) = self.scale_overrides.get(service_name) {
			return n as usize;
		}
		declared_replicas(service)
	}

	pub(in crate::backends::engine) fn container_name(
		&self,
		service_name: &str,
		service: &Service,
	) -> String {
		service
			.container_name
			.clone()
			.unwrap_or_else(|| format!("{}-{}", self.project, service_name))
	}

	/// Container names for `service` at exactly `count` replicas.
	///
	/// The auto-generated name is **always** index-suffixed (`{project}-{svc}-1`
	/// even for a single replica), matching docker-compose and podman-compose,
	/// which never expose a bare, unnumbered container name. An explicit
	/// `container_name:` is honoured verbatim at a single replica (and
	/// `{name}-1..-N` only when forced past one), since the user asked for that
	/// exact name. A `count` of 0 (`--scale svc=0`) yields no names.
	pub(in crate::backends::engine) fn replica_names_for(
		&self,
		service_name: &str,
		service: &Service,
		count: usize,
	) -> Vec<String> {
		match &service.container_name {
			Some(explicit) if count <= 1 => {
				if count == 0 {
					Vec::new()
				} else {
					vec![explicit.clone()]
				}
			}
			Some(explicit) => (1..=count).map(|i| format!("{explicit}-{i}")).collect(),
			None => {
				let base = format!("{}-{}", self.project, service_name);
				(1..=count).map(|i| format!("{base}-{i}")).collect()
			}
		}
	}

	pub(in crate::backends::engine) fn replica_names(
		&self,
		service_name: &str,
		service: &Service,
	) -> Vec<String> {
		let replicas = self.resolve_replicas(service_name, service);
		self.replica_names_for(service_name, service, replicas)
	}

	pub(in crate::backends::engine) fn first_replica_name(
		&self,
		service_name: &str,
		service: &Service,
	) -> String {
		// Falls back to the bare base only when the service resolves to zero
		// replicas (`--scale svc=0`), so callers that cannot represent "no
		// container" still get a stable, addressable name.
		self.replica_names(service_name, service)
			.into_iter()
			.next()
			.unwrap_or_else(|| self.container_name(service_name, service))
	}

	/// Resolve the container name for a service replica from the statically
	/// derived names: the 1-based `--index` when given (erroring if out of
	/// range), else the first replica.
	///
	/// Prefer [`Engine::live_replica_name_at`] for the replica-targeting
	/// commands (`exec`, `cp`): the static names reflect only the compose
	/// `scale:`/`deploy.replicas` (plus a `--scale` on the *current* invocation),
	/// so a later `cp`/`exec` would not see replicas created by a prior
	/// `up --scale`. This variant stays for callers that cannot await.
	pub(in crate::backends::engine) fn replica_name_at(
		&self,
		service_name: &str,
		service: &Service,
		index: Option<u32>,
	) -> Result<String> {
		let names = self.replica_names(service_name, service);
		let base = self.container_name(service_name, service);
		resolve_replica_name(service_name, &base, &names, index)
	}

	/// Resolve the container name for a service replica against the *running*
	/// scale: the replicas Podman actually has (matched by the `podup.service`
	/// label), falling back to the statically derived names before anything is
	/// created. `--index n` therefore targets replica `n` even when it was
	/// created by an earlier `up --scale`/`scale` rather than the current
	/// invocation, matching `docker compose cp/exec --index`. Shared by the
	/// replica-targeting commands (`exec`, `cp`).
	///
	/// Reads off the bulk project listing ([`Engine::live_project_replicas_sorted`])
	/// instead of issuing a per-service container-list round-trip (#1445):
	/// one shared GET powers the rest of the per-replica query paths
	/// (`port`, `logs`) when a command needs more than one of them.
	pub(in crate::backends::engine) async fn live_replica_name_at(
		&self,
		service_name: &str,
		service: &Service,
		index: Option<u32>,
	) -> Result<String> {
		let live_by_service = self.live_project_replicas_sorted().await?;
		let names = match live_by_service.get(service_name) {
			Some(names) if !names.is_empty() => names.clone(),
			// Service has no live container yet, so fall back to the static
			// compose names so a never-created service still has an
			// addressable replica.
			_ => self.replica_names(service_name, service),
		};
		let base = self.container_name(service_name, service);
		resolve_replica_name(service_name, &base, &names, index)
	}
}

use crate::engine::declared_replicas;

#[cfg(test)]
#[path = "replicas_tests.rs"]
mod tests;
