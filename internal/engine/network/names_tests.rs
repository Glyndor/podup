use super::*;
use crate::compose::types::{ComposeFile, NetworkConfig};

fn empty_file() -> ComposeFile {
	ComposeFile::default()
}

fn file_with_named_network(key: &str, name: &str) -> ComposeFile {
	let cfg = NetworkConfig {
		name: Some(name.to_string()),
		..Default::default()
	};
	let mut file = empty_file();
	file.networks.insert(key.to_string(), Some(cfg));
	file
}

#[test]
fn resolve_network_name_key_not_found_prefixes_project() {
	let file = empty_file();
	assert_eq!(resolve_network_name("mynet", &file, "proj"), "proj_mynet");
}

#[test]
fn resolve_network_name_uses_config_name_over_prefix() {
	let file = file_with_named_network("mynet", "custom-net-name");
	assert_eq!(
		resolve_network_name("mynet", &file, "proj"),
		"custom-net-name"
	);
}

#[test]
fn resolve_network_name_external_uses_key_not_prefix() {
	let cfg = NetworkConfig {
		external: Some(true),
		..Default::default()
	};
	let mut file = empty_file();
	file.networks.insert("shared".to_string(), Some(cfg));
	assert_eq!(resolve_network_name("shared", &file, "proj"), "shared");
}
