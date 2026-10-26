//! The per-service network configuration block: gw_priority, mac_address,
//! link_local_ips and interface_name.
//!
//! Split out of `tests/compose/parsing/parse/coverage/network_volume/mod.rs`; the imports are
//! inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// Network service config: gw_priority, mac_address, link_local_ips,
//                         interface_name
// ---------------------------------------------------------------------------

#[test]
fn network_service_gw_priority() {
	let yaml = r#"
networks:
  frontend:
services:
  app:
    image: alpine
    networks:
      frontend:
        gw_priority: 100
"#;
	let file = parse_str(yaml).unwrap();
	let cfg = file.services["app"]
		.networks
		.config_for("frontend")
		.unwrap();
	assert_eq!(cfg.gw_priority, Some(100));
}

#[test]
fn network_service_mac_address() {
	let yaml = r#"
networks:
  net:
services:
  app:
    image: alpine
    networks:
      net:
        mac_address: "02:42:ac:11:00:02"
"#;
	let file = parse_str(yaml).unwrap();
	let cfg = file.services["app"].networks.config_for("net").unwrap();
	assert_eq!(cfg.mac_address.as_deref(), Some("02:42:ac:11:00:02"));
}

#[test]
fn network_service_link_local_ips() {
	let yaml = r#"
networks:
  net:
services:
  app:
    image: alpine
    networks:
      net:
        link_local_ips:
          - 169.254.8.1
"#;
	let file = parse_str(yaml).unwrap();
	let cfg = file.services["app"].networks.config_for("net").unwrap();
	assert_eq!(cfg.link_local_ips.len(), 1);
	assert_eq!(cfg.link_local_ips[0], "169.254.8.1");
}

#[test]
fn network_service_interface_name() {
	let yaml = r#"
networks:
  net:
services:
  app:
    image: alpine
    networks:
      net:
        interface_name: eth0
"#;
	let file = parse_str(yaml).unwrap();
	let cfg = file.services["app"].networks.config_for("net").unwrap();
	assert_eq!(cfg.interface_name.as_deref(), Some("eth0"));
}
