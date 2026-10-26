//! Fields shared by every container in a pod.
//!
//! A pod-level field is safe only when every service declares the same value.
//! Conflicting values stay on the individual containers and produce a warning.

use crate::compose::types::ComposeFile;

use super::super::super::render::sanitize_value;

use super::super::shared_values::{host_values, hostname, network_values, shm_size, userns};
use super::super::{quote_podman_arg_value, Section};

/// A compose field that can move from a container unit to its pod unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in super::super) enum PodField {
	/// `hostname`.
	HostName,
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
	/// `shm_size`.
	ShmSize,
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

/// The pod-wide values agreed by every service, plus any conflicts.
pub(crate) struct PodFields {
	hostname: Agreement<String>,
	dns: Agreement<Vec<String>>,
	dns_option: Agreement<Vec<String>>,
	dns_search: Agreement<Vec<String>>,
	extra_hosts: Agreement<Vec<String>>,
	userns: Agreement<String>,
	shm_size: Agreement<String>,
	ipv4: Agreement<String>,
	ipv6: Agreement<String>,
	aliases: Agreement<Vec<String>>,
}

impl PodFields {
	/// Derive the shared values from every service in one compose file.
	pub(crate) fn new(file: &ComposeFile) -> Self {
		let hosts: Vec<_> = file.services.values().map(host_values).collect();
		let networks: Vec<_> = file.services.values().map(network_values).collect();
		Self {
			hostname: agree_scalar(
				file.services
					.values()
					.map(|service| hostname(service).map(str::to_owned)),
			),
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
			shm_size: agree_scalar(
				file.services
					.values()
					.map(|service| shm_size(service).map(str::to_owned)),
			),
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
			PodField::HostName => matches!(&self.hostname, Agreement::Agreed(_)),
			PodField::Dns => matches!(&self.dns, Agreement::Agreed(_)),
			PodField::DnsOption => matches!(&self.dns_option, Agreement::Agreed(_)),
			PodField::DnsSearch => matches!(&self.dns_search, Agreement::Agreed(_)),
			PodField::AddHost => matches!(&self.extra_hosts, Agreement::Agreed(_)),
			PodField::UserNs => matches!(&self.userns, Agreement::Agreed(_)),
			PodField::ShmSize => matches!(&self.shm_size, Agreement::Agreed(_)),
			PodField::Ip => matches!(&self.ipv4, Agreement::Agreed(_)),
			PodField::Ip6 => matches!(&self.ipv6, Agreement::Agreed(_)),
			PodField::NetworkAlias => matches!(&self.aliases, Agreement::Agreed(_)),
		}
	}

	/// Return one warning for every field the services disagree on.
	pub(crate) fn warnings(&self) -> Vec<String> {
		let mut warnings = Vec::new();
		let mut push = |field: &str, reason: Option<&str>| {
			if let Some(reason) = reason {
				warnings.push(format!(
					"x-podman-pod: {field}: {reason}; the value stays on each container"
				));
			}
		};
		push("hostname", self.hostname.warning_reason());
		push("dns", self.dns.warning_reason());
		push("dns_opt", self.dns_option.warning_reason());
		push("dns_search", self.dns_search.warning_reason());
		push("extra_hosts", self.extra_hosts.warning_reason());
		push("userns_mode", self.userns.warning_reason());
		push("shm_size", self.shm_size.warning_reason());
		push("networks.*.ipv4_address", self.ipv4.warning_reason());
		push("networks.*.ipv6_address", self.ipv6.warning_reason());
		push("networks.*.aliases", self.aliases.warning_reason());
		warnings
	}

	/// Emit every agreed field on the pod unit.
	pub(crate) fn emit(&self, pod: &mut Section) {
		if let Agreement::Agreed(value) = &self.hostname {
			add_podman_arg(pod, "--hostname", value);
		}
		if let Agreement::Agreed(values) = &self.dns {
			for value in values {
				add_podman_arg(pod, "--dns", value);
			}
		}
		if let Agreement::Agreed(values) = &self.dns_option {
			for value in values {
				add_podman_arg(pod, "--dns-option", value);
			}
		}
		if let Agreement::Agreed(values) = &self.dns_search {
			for value in values {
				add_podman_arg(pod, "--dns-search", value);
			}
		}
		if let Agreement::Agreed(values) = &self.extra_hosts {
			for value in values {
				add_podman_arg(pod, "--add-host", value);
			}
		}
		if let Agreement::Agreed(value) = &self.userns {
			add_podman_arg(pod, "--userns", value);
		}
		if let Agreement::Agreed(value) = &self.shm_size {
			add_podman_arg(pod, "--shm-size", value);
		}
		if let Agreement::Agreed(value) = &self.ipv4 {
			add_podman_arg(pod, "--ip", value);
		}
		if let Agreement::Agreed(value) = &self.ipv6 {
			add_podman_arg(pod, "--ip6", value);
		}
		if let Agreement::Agreed(values) = &self.aliases {
			for value in values {
				add_podman_arg(pod, "--network-alias", value);
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

/// Append one quoted Podman argument to a pod unit.
fn add_podman_arg(pod: &mut Section, flag: &str, value: &str) {
	let sanitized = sanitize_value(value);
	// Native container directives lose boundary whitespace during unit parsing.
	// Host entries and aliases already use quoted arguments on the container.
	let value = match flag {
		"--add-host" | "--network-alias" => sanitized.as_str(),
		_ => sanitized.trim(),
	};
	pod.add(
		"PodmanArgs",
		format!("{flag}={}", quote_podman_arg_value(value)),
	);
}
