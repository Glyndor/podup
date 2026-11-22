//! Fields shared by every container in a pod.
//!
//! Most pod-level fields are safe only when every service declares the same
//! value. Conflicting values stay on the individual containers and produce a
//! warning. Shared memory is different: Podman applies the pod's size to
//! every member, so the largest declared size wins.

use crate::compose::types::ComposeFile;

use super::super::super::render::sanitize_value;

use super::super::shared_values::{host_values, network_values, userns};
use super::super::Section;

/// A compose field that can move from a container unit to its pod unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in super::super) enum PodField {
	/// `dns`.
	Dns,
	/// `dns_opt`.
	DnsOption,
	/// `dns_search`.
	DnsSearch,
	/// `extra_hosts`.
	AddHost,
	/// `userns_mode`.
	UserNs,
	/// `networks.*.ipv4_address`.
	Ip,
	/// `networks.*.ipv6_address`.
	Ip6,
	/// `networks.*.aliases`.
	NetworkAlias,
}

/// The state of one field across all services in a project.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Agreement<T> {
	/// No service declares the field.
	Unset,
	/// Every service declares the same value.
	Agreed(T),
	/// Services declare different values.
	Conflicting,
	/// The value cannot preserve its meaning at pod level.
	Unshareable(&'static str),
}

/// Compare optional values from every service.
fn agree<T: Clone + Eq>(values: impl IntoIterator<Item = Option<T>>) -> Agreement<T> {
	let mut values = values.into_iter();
	let Some(first) = values.next() else {
		return Agreement::Unset;
	};
	match first {
		None => {
			if values.any(|value| value.is_some()) {
				Agreement::Conflicting
			} else {
				Agreement::Unset
			}
		}
		Some(first) => {
			for value in values {
				if value.as_ref() != Some(&first) {
					return Agreement::Conflicting;
				}
			}
			Agreement::Agreed(first)
		}
	}
}

/// Agree on a scalar that the pod flag would carry through argument unquoting.
/// A double quote inside the value is read differently by the native container
/// directive (scalar lookup drops boundary quotes) than by a quoted pod argument,
/// so such a value stays on the containers.
fn agree_scalar(values: impl IntoIterator<Item = Option<String>>) -> Agreement<String> {
	match agree(values) {
		Agreement::Agreed(value) if value.contains('"') => {
			Agreement::Unshareable("the value contains a double quote")
		}
		other => other,
	}
}

/// Treat an empty list as an absent value.
fn optional_list(values: Vec<String>) -> Option<Vec<String>> {
	(!values.is_empty()).then_some(values)
}

/// Native DNS reset entries cannot be translated into equivalent CLI flags.
fn agree_dns(values: impl IntoIterator<Item = Option<Vec<String>>>) -> Agreement<Vec<String>> {
	let values: Vec<_> = values.into_iter().collect();
	if values
		.iter()
		.flatten()
		.flatten()
		.any(|value| sanitize_value(value).trim().is_empty())
	{
		Agreement::Unshareable("DNS entries must not be empty or blank")
	} else {
		agree(values)
	}
}

/// Global address flags are only valid for one pod network attachment.
fn agree_address(
	file: &ComposeFile,
	values: impl IntoIterator<Item = Option<String>>,
) -> Agreement<String> {
	let values: Vec<_> = values.into_iter().collect();
	if values.iter().any(Option::is_some)
		&& (file.networks.len() != 1 || crate::engine::pod_network_mode(file).is_some())
	{
		Agreement::Unshareable("static pod addresses require exactly one declared network")
	} else {
		agree(values)
	}
}

/// The pod-wide values derived from every service, plus any conflicts.
pub(crate) struct PodFields {
	dns: Agreement<Vec<String>>,
	dns_option: Agreement<Vec<String>>,
	dns_search: Agreement<Vec<String>>,
	extra_hosts: Agreement<Vec<String>>,
	userns: Agreement<String>,
	shm_size: crate::engine::PodShmSize,
	ipv4: Agreement<String>,
	ipv6: Agreement<String>,
	aliases: Agreement<Vec<String>>,
}

impl PodFields {
	/// Derive the shared values from every service in one compose file.
	pub(crate) fn new(file: &ComposeFile) -> Self {
		let hosts: Vec<_> = file.services.values().map(host_values).collect();
		let networks: Vec<_> = file.services.values().map(network_values).collect();
		let shm_size = crate::engine::pod_shm_size(file);
		Self {
			dns: agree_dns(hosts.iter().map(|values| optional_list(values.dns.clone()))),
			dns_option: agree_dns(
				hosts
					.iter()
					.map(|values| optional_list(values.dns_option.clone())),
			),
			dns_search: agree_dns(
				hosts
					.iter()
					.map(|values| optional_list(values.dns_search.clone())),
			),
			extra_hosts: agree(
				hosts
					.iter()
					.map(|values| optional_list(values.extra_hosts.clone())),
			),
			userns: agree_scalar(
				file.services
					.values()
					.map(|service| userns(service).map(str::to_owned)),
			),
			shm_size,
			ipv4: agree_address(file, networks.iter().map(|values| values.ipv4.clone())),
			ipv6: agree_address(file, networks.iter().map(|values| values.ipv6.clone())),
			aliases: agree(
				networks
					.iter()
					.map(|values| optional_list(values.aliases.clone())),
			),
		}
	}

	/// Whether one field can be emitted on the pod instead of its containers.
	pub(in super::super) fn owns(&self, field: PodField) -> bool {
		match field {
			PodField::Dns => matches!(&self.dns, Agreement::Agreed(_)),
			PodField::DnsOption => matches!(&self.dns_option, Agreement::Agreed(_)),
			PodField::DnsSearch => matches!(&self.dns_search, Agreement::Agreed(_)),
			PodField::AddHost => matches!(&self.extra_hosts, Agreement::Agreed(_)),
			PodField::UserNs => matches!(&self.userns, Agreement::Agreed(_)),
			PodField::Ip => matches!(&self.ipv4, Agreement::Agreed(_)),
			PodField::Ip6 => matches!(&self.ipv6, Agreement::Agreed(_)),
			PodField::NetworkAlias => matches!(&self.aliases, Agreement::Agreed(_)),
		}
	}

	/// Return one warning for every field the services disagree on.
	///
	/// Shared memory reports the selected size because its value still moves
	/// to the pod; the other conflicting fields stay on their containers.
	pub(crate) fn warnings(&self) -> Vec<String> {
		let mut warnings = Vec::new();
		if let Some(warning) = &self.shm_size.warning {
			warnings.push(warning.clone());
		}
		let mut push = |field: &str, reason: Option<&str>| {
			if let Some(reason) = reason {
				warnings.push(format!(
					"x-podman-pod: {field}: {reason}; the value stays on each container"
				));
			}
		};
		push("dns", self.dns.warning_reason());
		push("dns_opt", self.dns_option.warning_reason());
		push("dns_search", self.dns_search.warning_reason());
		push("extra_hosts", self.extra_hosts.warning_reason());
		push("userns_mode", self.userns.warning_reason());
		push("networks.*.ipv4_address", self.ipv4.warning_reason());
		push("networks.*.ipv6_address", self.ipv6.warning_reason());
		push("networks.*.aliases", self.aliases.warning_reason());
		warnings
	}

	/// Emit every pod-level field on the pod unit.
	pub(crate) fn emit(&self, pod: &mut Section) {
		if let Agreement::Agreed(values) = &self.dns {
			for value in values {
				add_trimmed(pod, "DNS", "--dns=", value);
			}
		}
		if let Agreement::Agreed(values) = &self.dns_option {
			for value in values {
				add_trimmed(pod, "DNSOption", "--dns-option=", value);
			}
		}
		if let Agreement::Agreed(values) = &self.dns_search {
			for value in values {
				add_trimmed(pod, "DNSSearch", "--dns-search=", value);
			}
		}
		if let Agreement::Agreed(values) = &self.extra_hosts {
			for value in values {
				pod.add_native("AddHost", "--add-host=", &sanitize_value(value));
			}
		}
		if let Agreement::Agreed(value) = &self.userns {
			add_trimmed(pod, "UserNS", "--userns=", value);
		}
		if let Some(bytes) = self.shm_size.bytes {
			pod.add("ShmSize", bytes.to_string());
		}
		if let Agreement::Agreed(value) = &self.ipv4 {
			add_trimmed(pod, "IP", "--ip=", value);
		}
		if let Agreement::Agreed(value) = &self.ipv6 {
			add_trimmed(pod, "IP6", "--ip6=", value);
		}
		if let Agreement::Agreed(values) = &self.aliases {
			for value in values {
				pod.add_native("NetworkAlias", "--network-alias=", &sanitize_value(value));
			}
		}
	}
}

impl<T> Agreement<T> {
	/// Explain why a value stays on its container.
	fn warning_reason(&self) -> Option<&str> {
		match self {
			Agreement::Conflicting => Some("services disagree"),
			Agreement::Unshareable(reason) => Some(reason),
			_ => None,
		}
	}
}

/// Append a pod value that was always written trimmed: these fields went
/// through `PodmanArgs=` with the boundary whitespace removed, which is also
/// what the native key does on the container side. Host entries and aliases
/// are not trimmed (they keep their spaces, see `Section::add_native`).
fn add_trimmed(pod: &mut Section, key: &str, arg: &str, value: &str) {
	pod.add_native(key, arg, sanitize_value(value).trim());
}
