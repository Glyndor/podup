//! How the container reaches its host: its devices, its extra host entries, and
//! the name servers it resolves against.

use crate::compose::types::Service;

use super::super::super::pod::{PodField, PodFields};
use super::super::super::shared_values::host_values;
use super::super::{quote_podman_arg_value, Section};

/// Emit one `AddDevice=` line per `devices:` entry, one `--add-host` podman arg
/// per `extra_hosts:` entry, and the DNS settings.
pub(in super::super) fn emit_host_config(
	service: &Service,
	in_pod: bool,
	pod_fields: &PodFields,
	container: &mut Section,
) {
	for dev in &service.devices {
		container.add("AddDevice", dev.clone());
	}
	let values = host_values(service);
	if !(in_pod && pod_fields.owns(PodField::AddHost)) {
		for host in &values.extra_hosts {
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
	}
	if !(in_pod && pod_fields.owns(PodField::Dns)) {
		for d in values.dns {
			container.add("DNS", d);
		}
	}
	if !(in_pod && pod_fields.owns(PodField::DnsSearch)) {
		for d in values.dns_search {
			container.add("DNSSearch", d);
		}
	}
	if !(in_pod && pod_fields.owns(PodField::DnsOption)) {
		for d in values.dns_option {
			container.add("DNSOption", d);
		}
	}
}
