//! What the process inside the container is allowed to do: the capabilities it
//! gains and drops, the namespaces and groups it runs under, and the
//! `security_opt:` entries.

use crate::compose::types::Service;

use super::super::pod_fields::{PodField, PodFields};
use super::super::shared_values::userns;
use super::{map_security_opt, quote_podman_arg_value, Section};

/// Emit one `AddCapability=`/`DropCapability=` line per `cap_add:`/`cap_drop:`
/// entry.
pub(super) fn emit_capabilities(service: &Service, container: &mut Section) {
	for cap in &service.cap_add {
		container.add("AddCapability", cap.clone());
	}
	for cap in &service.cap_drop {
		container.add("DropCapability", cap.clone());
	}
}

/// Emit the `UserNS=` line for a `userns_mode:`.
pub(super) fn emit_userns(
	service: &Service,
	in_pod: bool,
	pod_fields: &PodFields,
	container: &mut Section,
) {
	if !(in_pod && pod_fields.owns(PodField::UserNs)) {
		if let Some(userns) = userns(service) {
			container.add("UserNS", userns.to_string());
		}
	}
}

/// Emit a `--group-add` podman arg per `group_add:` entry.
pub(super) fn emit_group_add(service: &Service, container: &mut Section) {
	for group in &service.group_add {
		// An empty entry (`- ""`) would render as `--group-add=`, which
		// podman rejects. The native `GroupAdd=` key omits empty values,
		// so the podman-args route has to skip them too.
		if group.is_empty() {
			continue;
		}
		// `GroupAdd=` was added to Quadlet in Podman 5.1.0. The supported
		// floor is 5.0, so route each entry through `PodmanArgs=` as
		// `--group-add`, like the memory/CPU limits. Quoted per #1734 for
		// the same reason as `mem_limit`.
		container.add(
			"PodmanArgs",
			format!("--group-add={}", quote_podman_arg_value(group)),
		);
	}
}

/// Map each `security_opt:` entry onto its Quadlet key or podman arg.
pub(super) fn emit_security_opts(
	service: &Service,
	name: &str,
	container: &mut Section,
	warnings: &mut Vec<String>,
) {
	for opt in &service.security_opt {
		map_security_opt(opt, container, name, warnings);
	}
}
