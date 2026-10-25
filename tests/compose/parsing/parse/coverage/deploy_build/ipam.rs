//! The `ipam` block of a network: driver, subnet, gateway, ip_range and
//! aux_addresses.
//!
//! Split out of `tests/compose/parsing/parse/coverage/deploy_build/mod.rs`; the imports are
//! inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// IPAM network config
// ---------------------------------------------------------------------------

#[test]
fn network_ipam_subnet_gateway() {
	let yaml = r#"
networks:
  backend:
    driver: bridge
    ipam:
      driver: default
      config:
        - subnet: 192.168.90.0/24
          gateway: 192.168.90.1
          ip_range: 192.168.90.128/25
"#;
	let file = parse_str(yaml).unwrap();
	let net = file.networks["backend"].as_ref().unwrap();
	let ipam = net.ipam.as_ref().unwrap();
	assert_eq!(ipam.driver.as_deref(), Some("default"));
	assert_eq!(ipam.config.len(), 1);
	assert_eq!(ipam.config[0].subnet.as_deref(), Some("192.168.90.0/24"));
	assert_eq!(ipam.config[0].gateway.as_deref(), Some("192.168.90.1"));
	assert_eq!(
		ipam.config[0].ip_range.as_deref(),
		Some("192.168.90.128/25")
	);
}

#[test]
fn network_ipam_aux_addresses() {
	let yaml = r#"
networks:
  mynet:
    ipam:
      config:
        - subnet: 172.16.238.0/24
          aux_addresses:
            host1: 172.16.238.5
"#;
	let file = parse_str(yaml).unwrap();
	let net = file.networks["mynet"].as_ref().unwrap();
	let pool = &net.ipam.as_ref().unwrap().config[0];
	assert_eq!(
		pool.aux_addresses.get("host1").map(|s| s.as_str()),
		Some("172.16.238.5")
	);
}
