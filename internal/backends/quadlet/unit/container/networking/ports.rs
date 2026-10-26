//! The ports a container publishes to the host, and the ones it only exposes.

use crate::compose::types::Service;
use crate::ports::parse_ports;

use super::super::{render_publish_port, Section};

/// Emit one `PublishPort=` line per mapped port, unless the pod owns the
/// published ports.
pub(in super::super) fn emit_ports(service: &Service, in_pod: bool, container: &mut Section) {
	// Ports are validated (range, format) before generation, so parsing succeeds
	// here. A malformed/out-of-range mapping is rejected at the command boundary
	// rather than re-emitted verbatim as an invalid `PublishPort=`; emitting the
	// raw string would produce a unit Quadlet/Podman would reject anyway.
	//
	// In pod mode the pod owns every published port (the container's own
	// `portmappings` would conflict with the pod's), so each `.container`
	// unit drops its `PublishPort=` lines and references the pod by name
	// further down.
	if !in_pod {
		if let Ok(ports) = parse_ports(&service.ports) {
			for p in ports {
				container.add("PublishPort", render_publish_port(&p));
			}
		}
	}
}

/// Emit one `ExposeHostPort=` line per `expose:` entry.
pub(in super::super) fn emit_expose(service: &Service, container: &mut Section) {
	for port in &service.expose {
		container.add("ExposeHostPort", port.clone());
	}
}
