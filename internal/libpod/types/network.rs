//! Podman libpod network API request and response types.

use std::collections::HashMap;

use serde::Serialize;

/// Request body for `POST /libpod/networks/create`.
#[derive(Serialize, Default)]
pub struct NetworkCreateRequest {
	/// Network name.
	pub name: String,

	/// Network driver (e.g. `bridge`, `macvlan`); the daemon default is used
	/// when omitted.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub driver: Option<String>,

	/// Whether the network is internal (no external/outbound connectivity).
	#[serde(skip_serializing_if = "Option::is_none")]
	pub internal: Option<bool>,

	/// Whether standalone containers may attach to the network.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub attachable: Option<bool>,

	/// Whether the built-in DNS resolver is enabled for the network.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub dns_enabled: Option<bool>,

	/// Whether IPv6 is enabled for the network.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub ipv6_enabled: Option<bool>,

	/// Network labels (key/value), including compose project/network labels.
	#[serde(skip_serializing_if = "HashMap::is_empty", default)]
	pub labels: HashMap<String, String>,

	/// Driver-specific options passed to the network driver (e.g. `mtu`,
	/// `com.docker.network.bridge.*`). Distinct from `ipam_options`.
	#[serde(skip_serializing_if = "HashMap::is_empty", default)]
	pub options: HashMap<String, String>,

	/// Options passed to the IPAM (IP address management) driver, e.g. the IPAM
	/// `driver` choice. Distinct from the driver-level `options`.
	#[serde(skip_serializing_if = "HashMap::is_empty", default)]
	pub ipam_options: HashMap<String, String>,

	/// Subnet/gateway definitions for the network; empty lets Podman auto-assign.
	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub subnets: Vec<Subnet>,
}

/// Subnet specification for network creation.
#[derive(Serialize, Default)]
pub struct Subnet {
	/// Subnet in CIDR notation (e.g. `10.89.0.0/24`).
	#[serde(skip_serializing_if = "Option::is_none")]
	pub subnet: Option<String>,

	/// Gateway IP address for the subnet; auto-assigned when omitted.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub gateway: Option<String>,

	/// Range of addresses within the subnet available for dynamic assignment.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub lease_range: Option<LeaseRange>,
}

/// Lease range for a subnet.
#[derive(Serialize)]
pub struct LeaseRange {
	/// First IP address (inclusive) in the assignable range.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub start_ip: Option<String>,

	/// Last IP address (inclusive) in the assignable range.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub end_ip: Option<String>,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn network_create_minimal() {
		let req = NetworkCreateRequest {
			name: "mynet".into(),
			dns_enabled: Some(true),
			..Default::default()
		};
		let v = serde_json::to_value(&req).unwrap();
		assert_eq!(v["name"], "mynet");
		assert_eq!(v["dns_enabled"], serde_json::json!(true));
		assert!(v.get("labels").is_none());
		assert!(v.get("subnets").is_none());
	}

	#[test]
	fn network_create_skips_empty_labels() {
		let req = NetworkCreateRequest {
			name: "n".into(),
			..Default::default()
		};
		let v = serde_json::to_value(&req).unwrap();
		assert!(v.get("labels").is_none());
	}

	#[test]
	fn subnet_with_gateway() {
		let s = Subnet {
			subnet: Some("10.89.0.0/24".into()),
			gateway: Some("10.89.0.1".into()),
			lease_range: None,
		};
		let v = serde_json::to_value(&s).unwrap();
		assert_eq!(v["subnet"], "10.89.0.0/24");
		assert_eq!(v["gateway"], "10.89.0.1");
	}
}
