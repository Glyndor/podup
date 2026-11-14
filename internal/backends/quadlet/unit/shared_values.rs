//! Compose values shared by a pod and its member containers.
//!
//! Both unit kinds must derive these values the same way. Keeping the mapping
//! here prevents a pod-level value and the container-level fallback from
//! drifting apart.

use std::collections::HashSet;

use crate::compose::types::Service;

use super::sorted_label_pairs;

/// DNS and host-entry values declared by one service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HostValues {
	/// Custom DNS servers.
	pub(super) dns: Vec<String>,
	/// DNS resolver options.
	pub(super) dns_option: Vec<String>,
	/// DNS search domains.
	pub(super) dns_search: Vec<String>,
	/// Extra host-to-address mappings.
	pub(super) extra_hosts: Vec<String>,
}

/// Network aliases and static addresses declared by one service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NetworkValues {
	/// Distinct, non-empty network aliases.
	pub(super) aliases: Vec<String>,
	/// First static IPv4 address across the service's networks.
	pub(super) ipv4: Option<String>,
	/// First static IPv6 address across the service's networks.
	pub(super) ipv6: Option<String>,
}

/// Return the container hostname declared by one service.
pub(super) fn hostname(service: &Service) -> Option<&str> {
	service.hostname.as_deref()
}

/// Return the DNS and host-entry values declared by one service.
pub(super) fn host_values(service: &Service) -> HostValues {
	HostValues {
		dns: service.dns.to_list(),
		dns_option: service.dns_opt.to_list(),
		dns_search: service.dns_search.to_list(),
		extra_hosts: service
			.extra_hosts
			.iter()
			.filter(|host| !host.is_empty())
			.cloned()
			.collect(),
	}
}

/// Return the user labels declared by one service, sorted by label name.
pub(super) fn label_values(service: &Service) -> Vec<(String, String)> {
	sorted_label_pairs(service.labels.to_map())
}

/// Return the user-namespace mode declared by one service.
pub(super) fn userns(service: &Service) -> Option<&str> {
	service.userns_mode.as_deref()
}

/// Return the shared-memory size declared by one service.
pub(super) fn shm_size(service: &Service) -> Option<&str> {
	service.shm_size.as_deref()
}

/// Return the network aliases and static addresses declared by one service.
///
/// Only the first static address of each family is usable because Quadlet has
/// no per-network IP scoping. Aliases are de-duplicated because Podman rejects
/// repeated alias flags on one container.
pub(super) fn network_values(service: &Service) -> NetworkValues {
	let mut aliases = Vec::new();
	let mut seen_aliases = HashSet::new();
	let mut ipv4 = None;
	let mut ipv6 = None;
	for net in service.networks.names() {
		let Some(config) = service.networks.config_for(&net) else {
			continue;
		};
		if let Some(config_aliases) = &config.aliases {
			// Emit each alias at most once: a repeated alias (within a network or across
			// networks) would produce duplicate `--network-alias=` flags on the same
			// argv, which podman rejects at container create.
			for alias in config_aliases {
				if !alias.is_empty() && seen_aliases.insert(alias.clone()) {
					aliases.push(alias.clone());
				}
			}
		}
		if ipv4.is_none() {
			ipv4 = config.ipv4_address.clone();
		}
		if ipv6.is_none() {
			ipv6 = config.ipv6_address.clone();
		}
	}
	NetworkValues {
		aliases,
		ipv4,
		ipv6,
	}
}
