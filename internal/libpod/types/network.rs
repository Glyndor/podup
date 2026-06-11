//! Podman libpod network API request and response types.

use std::collections::HashMap;

use serde::Serialize;

/// Request body for `POST /libpod/networks/create`.
#[derive(Serialize, Default)]
pub struct NetworkCreateRequest {
	pub name: String,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub driver: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub internal: Option<bool>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub attachable: Option<bool>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub dns_enabled: Option<bool>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub ipv6_enabled: Option<bool>,

	#[serde(skip_serializing_if = "HashMap::is_empty", default)]
	pub labels: HashMap<String, String>,

	#[serde(skip_serializing_if = "HashMap::is_empty", default)]
	pub options: HashMap<String, String>,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub subnets: Vec<Subnet>,
}

/// Subnet specification for network creation.
#[derive(Serialize, Default)]
pub struct Subnet {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub subnet: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub gateway: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub lease_range: Option<LeaseRange>,
}

/// Lease range for a subnet.
#[derive(Serialize)]
pub struct LeaseRange {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub start_ip: Option<String>,

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
