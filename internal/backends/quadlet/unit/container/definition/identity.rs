//! The name a container runs under and the pod it joins.

use crate::compose::types::Service;

use super::super::{safe_unit_stem, Section};

/// The name the container of `service` runs under: its `container_name:`, else
/// `{project}-{name}`. One definition, shared by the unit writer and by the
/// export of jobs, which has to know which names are taken.
pub(crate) fn container_name(project: &str, name: &str, service: &Service) -> String {
	service
		.container_name
		.clone()
		.unwrap_or_else(|| format!("{project}-{name}"))
}

/// Emit `ContainerName=` and, in pod mode, the `Pod=` reference the container
/// is launched under.
pub(in super::super) fn emit_identity(
	project: &str,
	name: &str,
	service: &Service,
	in_pod: bool,
	container: &mut Section,
) {
	// Default the container name to `{project}-{service}`, matching how `up`
	// names containers. Without the project prefix the unit would create a
	// container called just `web`, colliding with any other project's `web`
	// service and diverging from the running-stack name. An explicit
	// `container_name:` still wins.
	container.add("ContainerName", container_name(project, name, service));
	// In pod mode every container joins the project pod; reference it by the
	// `.pod` unit's stem (which is the project name). Without this the
	// container would launch outside the pod.
	if in_pod {
		container.add("Pod", format!("{}.pod", safe_unit_stem(project)));
	}
}
