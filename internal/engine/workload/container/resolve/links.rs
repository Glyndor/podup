use crate::compose::types::{ComposeFile, Service};

/// Resolve a service's `links` to concrete container references.
///
/// A compose `links:` entry names a sibling service; it is rewritten to that
/// service's container name with the service name kept as the network alias
/// (`{container}:{alias}`), so the linked container is reachable by the compose
/// service name. `external_links` reference containers outside the project and
/// are passed through verbatim.
pub(in crate::engine::workload::container) fn resolve_links(
	service: &Service,
	file: &ComposeFile,
	project: &str,
) -> Vec<String> {
	let mut links: Vec<String> = service
		.links
		.iter()
		.map(|link| {
			let (target, alias) = link.split_once(':').unwrap_or((link, link));
			let container = file
				.services
				.get(target)
				.map(|svc| {
					svc.container_name
						.clone()
						// Auto-generated container names are always index-suffixed;
						// a link/volumes_from references the first replica.
						.unwrap_or_else(|| format!("{project}-{target}-1"))
				})
				.unwrap_or_else(|| target.to_string());
			format!("{container}:{alias}")
		})
		.collect();
	links.extend(service.external_links.iter().cloned());
	links
}

/// Resolve a service's `volumes_from` entries to concrete container references.
///
/// libpod's `SpecGenerator.volumes_from` expects container names, but compose
/// `volumes_from:` names sibling services. Each entry may be `<service>`,
/// `service:<name>`, or `container:<name>`, any of which may carry a trailing
/// `:ro`/`:rw` access-mode suffix. The bare-service and `service:` forms are
/// rewritten to the referenced service's container name (an explicit
/// `container_name:` is honoured, otherwise the first replica
/// `{project}-{service}-1`), preserving
/// the access mode. The `container:` form already names a container outside the
/// project and is passed through verbatim, as is any service name that is not
/// declared in this compose file.
pub(in crate::engine::workload::container) fn resolve_volumes_from(
	service: &Service,
	file: &ComposeFile,
	project: &str,
) -> Vec<String> {
	service
		.volumes_from
		.iter()
		.map(|entry| {
			let (reference, mode) = crate::compose::dependencies::parse_volumes_from_entry(entry);
			let resolved = match reference {
				crate::compose::dependencies::VolumesFromRef::Container(name) => {
					// Already a concrete container outside the project: pass through.
					name.to_string()
				}
				crate::compose::dependencies::VolumesFromRef::Service(target) => file
					.services
					.get(target)
					.map(|svc| {
						svc.container_name
							.clone()
							// Auto-generated container names are always index-suffixed;
							// a link/volumes_from references the first replica.
							.unwrap_or_else(|| format!("{project}-{target}-1"))
					})
					// Unknown service: leave the reference untouched.
					.unwrap_or_else(|| target.to_string()),
			};
			match mode {
				crate::compose::dependencies::VolumesFromMode::Default => resolved,
				crate::compose::dependencies::VolumesFromMode::Explicit(mode) => {
					format!("{resolved}:{mode}")
				}
			}
		})
		.collect()
}
