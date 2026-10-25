//! Network maps: the per-service map form and the Podman network modes.
//!
//! Split out of `tests/compose/parsing/parse/fields/extends_build/mod.rs`; the imports are
//! inherited via `use super::*`.

use super::*;

#[test]
fn network_per_service_map_form() {
	let yaml = r#"
networks:
  frontend:
  backend:
services:
  app:
    image: alpine
    networks:
      frontend:
        aliases: [web, www]
        ipv4_address: 172.16.238.10
        ipv6_address: 2001:db8::10
      backend: ~
"#;
	let file = parse_str(yaml).unwrap();
	let names = file.services["app"].networks.names();
	assert!(names.contains(&"frontend".to_string()));
	assert!(names.contains(&"backend".to_string()));
	let cfg = file.services["app"]
		.networks
		.config_for("frontend")
		.expect("frontend cfg");
	assert_eq!(cfg.aliases.as_ref().unwrap().len(), 2);
	assert_eq!(cfg.ipv4_address.as_deref(), Some("172.16.238.10"));
	assert_eq!(cfg.ipv6_address.as_deref(), Some("2001:db8::10"));
}

#[test]
fn network_mode_slirp4netns() {
	let yaml = "services:\n  app:\n    image: alpine\n    network_mode: slirp4netns\n";
	assert_eq!(
		parse_str(yaml).unwrap().services["app"]
			.network_mode
			.as_deref(),
		Some("slirp4netns")
	);
}

#[test]
fn network_mode_pasta() {
	let yaml = "services:\n  app:\n    image: alpine\n    network_mode: pasta\n";
	assert_eq!(
		parse_str(yaml).unwrap().services["app"]
			.network_mode
			.as_deref(),
		Some("pasta")
	);
}

#[test]
fn network_mode_ns_path() {
	let yaml =
		"services:\n  app:\n    image: alpine\n    network_mode: \"ns:/run/netns/mynamespace\"\n";
	assert_eq!(
		parse_str(yaml).unwrap().services["app"]
			.network_mode
			.as_deref(),
		Some("ns:/run/netns/mynamespace")
	);
}
