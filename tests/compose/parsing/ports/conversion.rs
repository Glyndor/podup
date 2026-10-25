use podup::compose::types::PortMapping;
use podup::ports::{parse_ports, to_libpod};

fn short(s: &str) -> PortMapping {
	PortMapping::Short(s.to_string())
}

#[test]
fn libpod_mapping_fields() {
	let ports = parse_ports(&[short("8080:80")]).unwrap();
	let mappings = to_libpod(&ports);
	assert_eq!(mappings.len(), 1);
	let m = &mappings[0];
	assert_eq!(m.container_port, 80);
	assert_eq!(m.host_port, Some(8080));
	assert_eq!(m.host_ip, "");
	assert_eq!(m.protocol, "tcp");
}

#[test]
fn libpod_udp_protocol() {
	let ports = parse_ports(&[short("514:514/udp")]).unwrap();
	let mappings = to_libpod(&ports);
	assert_eq!(mappings.len(), 1);
	assert_eq!(mappings[0].protocol, "udp");
	assert_eq!(mappings[0].container_port, 514);
}

#[test]
fn libpod_range_produces_multiple_entries() {
	let ports = parse_ports(&[short("8000-8001:8000-8001")]).unwrap();
	let mappings = to_libpod(&ports);
	assert_eq!(mappings.len(), 2);
	let cports: Vec<u16> = mappings.iter().map(|m| m.container_port).collect();
	assert!(cports.contains(&8000));
	assert!(cports.contains(&8001));
}
