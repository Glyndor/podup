//! Effective `depends_on` for a service: explicit entries plus the implicit
//! entries docker compose derives from references that need the referenced
//! container to exist first.
//!
//! `volumes_from: [data]`, `links: [data:alias]` and
//! `network_mode`/`ipc`/`pid`/`uts` = `service:data` all reach for a sibling
//! container's volumes, hostname, or namespace; starting the dependent before
//! that container exists is an immediate podman failure
//! (`looking up container to share net namespace with: no container with name or ID ...`).
//!
//! docker compose turns each such reference into an implicit `depends_on`
//! entry when it loads the file. We do not. We compute it on demand.
//! The reason is the `config_hash` in
//! `internal/engine/container/resolve.rs`: it serializes the whole
//! `Service`, `depends_on` included, into the hash that drives the
//! recreate-on-change decision. Mutating `Service::depends_on` at load
//! time would change the hash of every existing service that uses one of
//! these references, and the next `up` after upgrading would destroy and
//! recreate those containers (their writable layer is lost). Computing the
//! effective list here means the on-disk YAML stays byte-identical and the
//! hash stays stable across the upgrade.

use indexmap::IndexMap;

use crate::compose::types::{DependsOn, DependsOnCondition, Service, ServiceCondition};

/// One parsed `volumes_from:` entry. Split off the optional trailing
/// access mode so the caller can reattach it after the service-name lookup.
/// `container:<name>` is never a service, so it is a separate variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VolumesFromRef<'a> {
	/// `name[:ro|rw]` or `service:name[:ro|rw]`: a sibling service in the
	/// same compose file. The lifetime points into the raw entry string.
	Service(&'a str),
	/// `container:name[:ro|rw]`: an existing container outside the project.
	/// The engine must look up an actual container by this name at run time.
	Container(&'a str),
}

/// Trailing `:ro`/`:rw` on a `volumes_from:` entry, kept aside so the
/// service-name rewrite below does not drop it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VolumesFromMode {
	/// No trailing mode; default behaviour.
	Default,
	/// Explicit `:ro` (read-only) or `:rw` (read-write) suffix.
	Explicit(&'static str),
}

/// Split a single `volumes_from:` entry into the referent and its optional
/// `:ro`/`:rw` access mode. `container:<name>` is recognised as a
/// non-service reference (a container outside the project) and returned in
/// [`VolumesFromRef::Container`]; the bare `<name>`, `service:<name>` and
/// `service:<name>:ro` forms are returned in [`VolumesFromRef::Service`].
/// The grammar is the one [`crate::engine::container::resolve::resolve_volumes_from`]
/// already accepts; this helper exists so the same parser feeds both the
/// container-name rewrite and the implicit-dependency computation, and a
/// future shape change cannot drift between them.
pub(crate) fn parse_volumes_from_entry(entry: &str) -> (VolumesFromRef<'_>, VolumesFromMode) {
	let (reference, mode) = match entry.rsplit_once(':') {
		Some((head, "ro")) => (head, VolumesFromMode::Explicit("ro")),
		Some((head, "rw")) => (head, VolumesFromMode::Explicit("rw")),
		_ => (entry, VolumesFromMode::Default),
	};
	let reference = if let Some(name) = reference.strip_prefix("container:") {
		VolumesFromRef::Container(name)
	} else {
		let target = reference.strip_prefix("service:").unwrap_or(reference);
		VolumesFromRef::Service(target)
	};
	(reference, mode)
}

/// The `depends_on` value the engine should actually use for `service`:
/// the user's explicit `depends_on` plus one implicit entry per reference
/// that needs the referenced container to exist first. Explicit entries
/// win: an entry the user wrote is never replaced, only augmented.
///
/// The implicit entries this adds:
/// `volumes_from: [name]` or `[name:ro]` or `[service:name:ro]` →
///   `name: { condition: service_started, required: true, restart: None }`
///   (`container:name` does not produce an entry, it is a container outside
///   the project; an unknown name produces no entry either, the engine
///   passes it through to Podman as a container name).
/// `links: [name]` or `[name:alias]` → `name: { restart: true, ... }`
///   (the alias is the part after the first `:` and is dropped here).
/// `network_mode`/`ipc`/`pid`/`uts` = `service:name` →
///   `name: { restart: true, ... }`.
pub(crate) fn effective_depends_on(
	service: &Service,
	services: &IndexMap<String, Service>,
) -> DependsOn {
	let implicit_names = implicit_names(service, services);
	if implicit_names.is_empty() {
		// Nothing to add. Return the user's explicit value byte-identical so
		// `config --hash` and the printed `depends_on:` output stay unchanged.
		return service.depends_on.clone();
	}
	let mut map: IndexMap<String, DependsOnCondition> = match &service.depends_on {
		DependsOn::Empty => IndexMap::new(),
		DependsOn::List(names) => names
			.iter()
			.map(|n| {
				(
					n.clone(),
					DependsOnCondition {
						condition: ServiceCondition::ServiceStarted,
						restart: None,
						required: None,
					},
				)
			})
			.collect(),
		DependsOn::Map(m) => m.clone(),
	};
	for (name, restart) in implicit_names {
		map.entry(name.clone())
			.and_modify(|existing| {
				if existing.condition == ServiceCondition::default() && restart {
					existing.restart = Some(true);
				}
			})
			.or_insert(DependsOnCondition {
				condition: ServiceCondition::ServiceStarted,
				restart: if restart { Some(true) } else { None },
				required: Some(true),
			});
	}
	DependsOn::Map(map)
}

/// Names this service references via `volumes_from`, `links` or
/// `service:X` namespace settings, each paired with whether the engine
/// should propagate restarts for it (true for `links` and the four
/// namespace keys; false for `volumes_from`, matching docker compose).
/// Undeclared names are skipped: an unknown `volumes_from` name is
/// passed through to Podman as a container name and is not a service in
/// this file.
fn implicit_names(service: &Service, services: &IndexMap<String, Service>) -> Vec<(String, bool)> {
	let mut out: Vec<(String, bool)> = Vec::new();
	for entry in &service.volumes_from {
		let (reference, _mode) = parse_volumes_from_entry(entry);
		if let VolumesFromRef::Service(name) = reference {
			if services.contains_key(name) && !out.iter().any(|(n, _)| n == name) {
				out.push((name.to_string(), false));
			}
		}
	}
	for link in &service.links {
		let target = link
			.split_once(':')
			.map_or(link.as_str(), |(head, _alias)| head);
		if services.contains_key(target) && !out.iter().any(|(n, _)| n == target) {
			out.push((target.to_string(), true));
		}
	}
	for value in [
		&service.network_mode,
		&service.ipc,
		&service.pid,
		&service.uts,
	]
	.into_iter()
	.flatten()
	{
		if let Some(target) = value.strip_prefix("service:") {
			if services.contains_key(target) && !out.iter().any(|(n, _)| n == target) {
				out.push((target.to_string(), true));
			}
		}
	}
	out
}

#[cfg(test)]
#[path = "dependencies_tests.rs"]
mod tests;
