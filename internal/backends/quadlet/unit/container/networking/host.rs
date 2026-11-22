//! How the container reaches its host: its devices, its extra host entries, and
//! the name servers it resolves against.

use crate::compose::types::Service;

use super::super::super::pod::{PodField, PodFields};
use super::super::super::shared_values::host_values;
use super::super::Section;

/// Emit one `AddDevice=` line per `devices:` entry, one `AddHost=` line
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
			// An empty entry (`- ""`) would render as a bare `AddHost=`, which
			// resets the list instead of adding a host. An entry that is empty
			// once the control characters are stripped, or that has a space at
			// either end, stays on `PodmanArgs=` (see `Section::add_native`).
			if host.is_empty() {
				continue;
			}
			container.add_native("AddHost", "--add-host=", host);
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
