use super::build_steps;
use crate::compose::parse_str;

#[test]
fn pre_start_does_not_inherit_ports_or_static_addresses() {
	let file = parse_str(
		r#"
services:
  web:
    image: base
    ports: ['8080:80']
    expose: ['81']
    mac_address: '02:00:00:00:00:01'
    networks:
      back:
        ipv4_address: 10.0.0.2
        ipv6_address: 'fd00::2'
        mac_address: '02:00:00:00:00:02'
        aliases: [web]
    pre_start:
      - command: init
      - ports: ['9090:90']
        expose: ['91']
        mac_address: '02:00:00:00:00:03'
        networks:
          back:
            ipv4_address: 10.0.0.3
            ipv6_address: 'fd00::3'
            mac_address: '02:00:00:00:00:04'
"#,
	)
	.unwrap();
	let steps = build_steps(&file.services["web"]);
	let first = &steps[0];
	assert!(first.ports.is_empty());
	assert!(first.expose.is_empty());
	assert!(first.mac_address.is_none());
	let network = first.networks.config_for("back").unwrap();
	assert!(network.ipv4_address.is_none());
	assert!(network.ipv6_address.is_none());
	assert!(network.mac_address.is_none());
	assert_eq!(network.aliases.as_ref().unwrap(), &["web"]);
	let second = &steps[1];
	assert_eq!(second.ports.len(), 1);
	assert_eq!(second.expose, ["91"]);
	assert_eq!(second.mac_address.as_deref(), Some("02:00:00:00:00:03"));
	let network = second.networks.config_for("back").unwrap();
	assert_eq!(network.ipv4_address.as_deref(), Some("10.0.0.3"));
	assert_eq!(network.ipv6_address.as_deref(), Some("fd00::3"));
	assert_eq!(network.mac_address.as_deref(), Some("02:00:00:00:00:04"));
}

#[test]
fn a_step_with_networks_does_not_inherit_the_network_mode() {
	let file = parse_str(
		r#"
services:
  web:
    image: base
    network_mode: host
    pre_start:
      - command: init
        networks: [back]
networks:
  back: {}
"#,
	)
	.unwrap();
	let steps = build_steps(&file.services["web"]);
	assert!(steps[0].network_mode.is_none());
	assert!(!steps[0].networks.names().is_empty());
}

#[test]
fn a_step_with_a_network_mode_does_not_inherit_the_networks() {
	let file = parse_str(
		r#"
services:
  web:
    image: base
    networks: [back]
    pre_start:
      - command: init
        network_mode: none
networks:
  back: {}
"#,
	)
	.unwrap();
	let steps = build_steps(&file.services["web"]);
	assert_eq!(steps[0].network_mode.as_deref(), Some("none"));
	assert!(steps[0].networks.names().is_empty());
}

#[test]
fn a_step_without_network_settings_inherits_the_service_network_mode() {
	let file = parse_str(
		r#"
services:
  web:
    image: base
    network_mode: host
    pre_start:
      - command: init
"#,
	)
	.unwrap();
	let steps = build_steps(&file.services["web"]);
	assert_eq!(steps[0].network_mode.as_deref(), Some("host"));
}
