//! An empty `networks:` is no `networks:`.
//!
//! `docker compose config` attaches a service whose `networks:` is `[]` or `{}`
//! to the project `default` network, the same as one that omits the key. Leaving
//! it empty gave the container no project network at all, so it could not reach
//! its siblings by name.

use super::{normalize_default_network, parse_str};

#[test]
fn an_empty_list_attaches_the_service_to_the_default_network() {
	let mut file = parse_str("services:\n  web:\n    image: x\n    networks: []\n").unwrap();
	normalize_default_network(&mut file);
	assert!(file.networks.contains_key("default"));
	assert_eq!(file.services["web"].networks.names(), vec!["default"]);
}

#[test]
fn an_empty_map_attaches_the_service_to_the_default_network() {
	let mut file = parse_str("services:\n  web:\n    image: x\n    networks: {}\n").unwrap();
	normalize_default_network(&mut file);
	assert!(file.networks.contains_key("default"));
	assert_eq!(file.services["web"].networks.names(), vec!["default"]);
}

#[test]
fn an_empty_list_does_not_override_network_mode() {
	let mut file =
		parse_str("services:\n  web:\n    image: x\n    network_mode: host\n    networks: []\n")
			.unwrap();
	normalize_default_network(&mut file);
	assert!(file.services["web"].networks.names().is_empty());
}

#[test]
fn a_named_network_is_left_alone() {
	let mut file =
		parse_str("services:\n  web:\n    image: x\n    networks: [front]\nnetworks:\n  front:\n")
			.unwrap();
	normalize_default_network(&mut file);
	assert_eq!(file.services["web"].networks.names(), vec!["front"]);
	assert!(!file.networks.contains_key("default"));
}
