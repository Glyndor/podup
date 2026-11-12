//! `config_hash` and replica counts: how many replicas a service runs is not a
//! property of any one container, so changing it must not recreate the
//! replicas that already exist (Compose starts the missing ones, or removes the
//! surplus, and leaves the rest alone).

use super::config_hash;
use crate::parse_str;
use std::collections::{BTreeMap, HashMap};

fn hash_of(service_body: &str) -> String {
	let file = parse_str(&format!("services:\n  web:\n    image: x\n{service_body}")).unwrap();
	let service = file.services.get("web").unwrap();
	config_hash(
		service,
		&file,
		"proj",
		std::path::Path::new("."),
		&HashMap::new(),
		&BTreeMap::new(),
	)
	.unwrap()
}

#[test]
fn changing_scale_does_not_change_the_hash() {
	assert_eq!(hash_of("    scale: 1\n"), hash_of("    scale: 3\n"));
}

#[test]
fn changing_deploy_replicas_does_not_change_the_hash() {
	assert_eq!(
		hash_of("    deploy:\n      replicas: 1\n"),
		hash_of("    deploy:\n      replicas: 3\n")
	);
}

#[test]
fn changing_the_command_still_changes_the_hash() {
	assert_ne!(
		hash_of("    scale: 2\n"),
		hash_of("    scale: 2\n    command: [\"sleep\", \"1\"]\n")
	);
}
