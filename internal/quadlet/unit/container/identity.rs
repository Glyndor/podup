//! The name a container runs under and the pod it joins.

use crate::compose::types::Service;

use super::{safe_unit_stem, Section};

/// Emit `ContainerName=` and, in pod mode, the `Pod=` reference the container
/// is launched under.
pub(super) fn emit_identity(
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
	container.add(
		"ContainerName",
		service
			.container_name
			.clone()
			.unwrap_or_else(|| format!("{project}-{name}")),
	);
	// In pod mode every container joins the project pod; reference it by the
	// `.pod` unit's stem (which is the project name). Without this the
	// container would launch outside the pod.
	if in_pod {
		container.add("Pod", format!("{}.pod", safe_unit_stem(project)));
	}
}
