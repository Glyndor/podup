//! What the process inside the container is allowed to do: the capabilities it
//! gains and drops, the namespaces and groups it runs under, and the
//! `security_opt:` entries.

use crate::compose::types::Service;

use super::super::super::pod::{PodField, PodFields};
use super::super::super::shared_values::userns;
use super::super::{map_security_opt, Section};

/// Emit one `AddCapability=`/`DropCapability=` line per `cap_add:`/`cap_drop:`
/// entry.
pub(in super::super) fn emit_capabilities(service: &Service, container: &mut Section) {
	for cap in &service.cap_add {
		container.add("AddCapability", cap.clone());
	}
	for cap in &service.cap_drop {
		container.add("DropCapability", cap.clone());
	}
}

/// Emit the `UserNS=` line for a `userns_mode:`.
pub(in super::super) fn emit_userns(
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

/// Emit a `GroupAdd=` line per `group_add:` entry.
pub(in super::super) fn emit_group_add(service: &Service, container: &mut Section) {
	for group in &service.group_add {
		// An empty entry (`- ""`) would render as a bare `GroupAdd=`, which
		// resets the list instead of adding a group.
		if group.is_empty() {
			continue;
		}
		container.add("GroupAdd", group.clone());
	}
}

/// Map each `security_opt:` entry onto its Quadlet key or podman arg.
pub(in super::super) fn emit_security_opts(
	service: &Service,
	name: &str,
	container: &mut Section,
	warnings: &mut Vec<String>,
) {
	for opt in &service.security_opt {
		map_security_opt(opt, container, name, warnings);
	}
}
