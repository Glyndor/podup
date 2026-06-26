//! Network configuration types for both top-level networks and per-service attachments.
//!
//! [`NetworkConfig`] describes a named network in the `networks:` top-level block.
//! [`ServiceNetworks`] is the per-service attachment — either a bare list of names
//! or a map with [`ServiceNetworkConfig`] options (aliases, IP, priority, etc.).

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::Labels;

/// `networks:` value at service level — absent, a bare list of network names, or a detailed map.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(untagged)]
pub enum ServiceNetworks {
	/// No networks declared.
	#[default]
	Empty,
	/// Short form: a bare list of network names.
	List(Vec<String>),
	/// Long form: per-network attachment options keyed by network name.
	Map(IndexMap<String, Option<ServiceNetworkConfig>>),
}

impl ServiceNetworks {
	/// Returns the names of all attached networks.
	pub fn names(&self) -> Vec<String> {
		match self {
			ServiceNetworks::Empty => vec![],
			ServiceNetworks::List(v) => v.clone(),
			ServiceNetworks::Map(m) => m.keys().cloned().collect(),
		}
	}

	/// Returns the attachment options for a network, if any are set.
	pub fn config_for(&self, name: &str) -> Option<&ServiceNetworkConfig> {
		match self {
			ServiceNetworks::Map(m) => m.get(name).and_then(|c| c.as_ref()),
			_ => None,
		}
	}
}

/// Per-network attachment options: aliases, static IPv4/IPv6, link-local addresses, and priority.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ServiceNetworkConfig {
	/// Additional network aliases the container is reachable by.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub aliases: Option<Vec<String>>,
	/// Static IPv4 address on this network.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub ipv4_address: Option<String>,
	/// Static IPv6 address on this network.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub ipv6_address: Option<String>,
	/// Link-local IP addresses for this attachment.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub link_local_ips: Vec<String>,
	/// Attachment priority used to order multiple networks.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub priority: Option<u32>,
	/// Static MAC address for this attachment.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub mac_address: Option<String>,
	/// Driver-specific options for this attachment.
	#[serde(default, skip_serializing_if = "HashMap::is_empty")]
	pub driver_opts: HashMap<String, String>,
	/// Priority used to select the default gateway among networks.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub gw_priority: Option<u32>,
	/// Name of the interface created inside the container.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub interface_name: Option<String>,
	/// Unknown keys preserved verbatim for round-tripping and forward-compat
	/// diagnostics.
	#[serde(flatten, default, skip_serializing_if = "indexmap::IndexMap::is_empty")]
	pub unknown: indexmap::IndexMap<String, serde_yaml::Value>,
}

/// Named network definition in the top-level `networks:` block.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[non_exhaustive]
pub struct NetworkConfig {
	/// Network driver name; the runtime default is used if absent.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub driver: Option<String>,
	/// Driver-specific options.
	#[serde(default, skip_serializing_if = "HashMap::is_empty")]
	pub driver_opts: HashMap<String, String>,
	/// Whether the network is externally managed and not created by podup.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub external: Option<bool>,
	/// Custom network name overriding the project-prefixed default.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
	/// Whether the network is isolated from external connectivity.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub internal: Option<bool>,
	/// Whether IPv6 is enabled on the network.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub enable_ipv6: Option<bool>,
	/// Whether IPv4 is enabled on the network.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub enable_ipv4: Option<bool>,
	/// Whether standalone containers may attach to the network.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub attachable: Option<bool>,
	/// IP address management configuration.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub ipam: Option<IpamConfig>,
	/// Labels applied to the network.
	#[serde(default)]
	pub labels: Labels,
	/// Unrecognized keys preserved verbatim for round-tripping.
	#[serde(flatten, default, skip_serializing_if = "IndexMap::is_empty")]
	pub unknown: IndexMap<String, serde_yaml::Value>,
}

/// `ipam:` block inside a top-level network definition.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct IpamConfig {
	/// IPAM driver name.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub driver: Option<String>,
	/// Subnet/range pool definitions.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub config: Vec<IpamPool>,
	/// Driver-specific IPAM options.
	#[serde(default, skip_serializing_if = "HashMap::is_empty")]
	pub options: HashMap<String, String>,
	/// Unrecognized keys preserved verbatim for round-tripping.
	#[serde(flatten, default, skip_serializing_if = "IndexMap::is_empty")]
	pub unknown: IndexMap<String, serde_yaml::Value>,
}

/// A single subnet/range entry within `ipam.config`.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct IpamPool {
	/// Subnet in CIDR notation.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub subnet: Option<String>,
	/// Gateway address for the subnet.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub gateway: Option<String>,
	/// Range within the subnet to allocate addresses from.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub ip_range: Option<String>,
	/// Reserved auxiliary addresses (`name -> address`).
	#[serde(default, skip_serializing_if = "HashMap::is_empty")]
	pub aux_addresses: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;
	use indexmap::IndexMap;

	// ServiceNetworks::names

	#[test]
	fn service_networks_empty_has_no_names() {
		assert!(ServiceNetworks::Empty.names().is_empty());
	}

	#[test]
	fn service_networks_list_returns_names() {
		let n = ServiceNetworks::List(vec!["front".into(), "back".into()]);
		assert_eq!(n.names(), vec!["front", "back"]);
	}

	#[test]
	fn service_networks_map_returns_keys() {
		let mut m = IndexMap::new();
		m.insert("front".to_string(), None);
		assert_eq!(ServiceNetworks::Map(m).names(), vec!["front"]);
	}

	// ServiceNetworks::config_for

	#[test]
	fn config_for_list_returns_none() {
		let n = ServiceNetworks::List(vec!["front".into()]);
		assert!(n.config_for("front").is_none());
	}

	#[test]
	fn config_for_map_with_none_config_returns_none() {
		let mut m = IndexMap::new();
		m.insert("front".to_string(), None::<ServiceNetworkConfig>);
		assert!(ServiceNetworks::Map(m).config_for("front").is_none());
	}

	#[test]
	fn config_for_map_with_config_returns_it() {
		let cfg = ServiceNetworkConfig {
			ipv4_address: Some("10.0.0.2".into()),
			..Default::default()
		};
		let mut m = IndexMap::new();
		m.insert("front".to_string(), Some(cfg));
		let nets = ServiceNetworks::Map(m);
		let result = nets.config_for("front");
		assert_eq!(result.unwrap().ipv4_address.as_deref(), Some("10.0.0.2"));
	}

	#[test]
	fn config_for_missing_key_returns_none() {
		let mut m = IndexMap::new();
		m.insert("front".to_string(), None::<ServiceNetworkConfig>);
		assert!(ServiceNetworks::Map(m).config_for("back").is_none());
	}
}
