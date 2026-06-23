//! Podman libpod volume API request and response types.

use std::collections::HashMap;

use serde::Serialize;

/// Request body for `POST /libpod/volumes/create`.
#[derive(Serialize, Default)]
pub struct VolumeCreateOptions {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub driver: Option<String>,

	// Podman's VolumeCreateOptions carries no json tags, so its body decodes by
	// Go field name (case-insensitively): the driver-options map is `Options`, not
	// `driver_opts`. Without this rename volume driver_opts (NFS/CIFS/tmpfs) are
	// silently dropped while name/driver/labels still match case-insensitively.
	#[serde(rename = "Options", skip_serializing_if = "HashMap::is_empty", default)]
	pub driver_opts: HashMap<String, String>,

	#[serde(skip_serializing_if = "HashMap::is_empty", default)]
	pub labels: HashMap<String, String>,
}

#[cfg(test)]
mod tests {
	use super::VolumeCreateOptions;

	#[test]
	fn driver_opts_serialize_as_options_not_driver_opts() {
		let mut opts = VolumeCreateOptions {
			driver: Some("local".to_string()),
			..Default::default()
		};
		opts.driver_opts
			.insert("type".to_string(), "nfs".to_string());
		let v = serde_json::to_value(&opts).unwrap();
		// Podman's VolumeCreateOptions has no json tag on the options map, so the
		// wire key must be `Options`; `driver_opts` would be silently dropped.
		assert!(v.get("Options").is_some(), "expected Options key: {v}");
		assert!(v.get("driver_opts").is_none(), "stale driver_opts key: {v}");
		assert_eq!(v["Options"]["type"], "nfs");
	}
}
