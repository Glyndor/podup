//! How the container reaches its host: its devices, its extra host entries, and
//! the name servers it resolves against.

use crate::compose::types::Service;

use super::{quote_podman_arg_value, Section};

/// Emit one `AddDevice=` line per `devices:` entry, one `--add-host` podman arg
/// per `extra_hosts:` entry, and the DNS settings.
pub(super) fn emit_host_config(service: &Service, container: &mut Section) {
	for dev in &service.devices {
		container.add("AddDevice", dev.clone());
	}
	for host in &service.extra_hosts {
		// An empty entry (`- ""`) would render as `--add-host=`, which
		// podman rejects with `error parsing additional short
		// argument`. The native `AddHost=` key omits empty values, so the
		// podman-args route has to skip them too.
		if host.is_empty() {
			continue;
		}
		// `AddHost=` was added to Quadlet in Podman 5.3.0. The supported
		// floor is 5.0, so route each entry through `PodmanArgs=` as
		// `--add-host`, like the memory/CPU limits: Quadlet sees no key
		// above the floor, podman sees the same flag. Quoted per #1734 for
		// the same reason as `mem_limit`: a value carrying whitespace
		// must not become two argv elements.
		container.add(
			"PodmanArgs",
			format!("--add-host={}", quote_podman_arg_value(host)),
		);
	}
	for d in service.dns.to_list() {
		container.add("DNS", d);
	}
	for d in service.dns_search.to_list() {
		container.add("DNSSearch", d);
	}
	for d in service.dns_opt.to_list() {
		container.add("DNSOption", d);
	}
}
