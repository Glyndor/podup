//! Replica resolution helpers (see `project/replicas.rs`)

use crate::compose::types::Service;

/// How many replicas a service declares in the file, before any `--scale` on
/// the current invocation. Split out of
/// [`Engine::resolve_replicas`](crate::engine::Engine::resolve_replicas) so a
/// caller with no `Engine` (autostart's start mode) reads the same rule
/// rather than restating it.
pub(crate) fn declared_replicas(service: &Service) -> usize {
	service
		.scale
		.or(service.deploy.as_ref().and_then(|d| d.replicas))
		.unwrap_or(1) as usize
}

/// The container name a service resolves to at exactly one replica.
///
/// This is
/// [`Engine::replica_names_for`](crate::engine::Engine::replica_names_for) at
/// `count == 1`, extracted so autostart's start mode names the container the
/// engine actually created instead of spelling the rule out a second time.
/// The two are pinned together by
/// `naming_agrees_with_the_engine_at_one_replica`; without that test this
/// would be a copy that drifts silently, and the unit would name a container
/// that does not exist.
pub(crate) fn sole_replica_name(project: &str, service_name: &str, service: &Service) -> String {
	match &service.container_name {
		Some(explicit) => explicit.clone(),
		None => format!("{project}-{service_name}-1"),
	}
}
