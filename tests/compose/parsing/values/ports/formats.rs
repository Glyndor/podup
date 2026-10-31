use podup::compose::types::PortMapping;
use podup::ports::parse_ports;

fn short(s: &str) -> PortMapping {
	PortMapping::Short(s.to_string())
}

#[test]
fn container_only() {
	let ports = parse_ports(&[short("80")]).unwrap();
	assert_eq!(ports[0].container_port, 80);
	assert_eq!(ports[0].protocol, "tcp");
	assert!(ports[0].host_port.is_none());
}

#[test]
fn host_colon_container() {
	let ports = parse_ports(&[short("8080:80")]).unwrap();
	assert_eq!(ports[0].container_port, 80);
	assert_eq!(ports[0].host_port, Some(8080));
}

#[test]
fn ip_host_container() {
	let ports = parse_ports(&[short("127.0.0.1:8080:80")]).unwrap();
	assert_eq!(ports[0].container_port, 80);
	assert_eq!(ports[0].host_port, Some(8080));
	assert_eq!(ports[0].host_ip, "127.0.0.1");
}

#[test]
fn udp_protocol() {
	assert_eq!(
		parse_ports(&[short("514:514/udp")]).unwrap()[0].protocol,
		"udp"
	);
}

#[test]
fn container_only_udp() {
	let ports = parse_ports(&[short("53/udp")]).unwrap();
	assert_eq!(ports[0].container_port, 53);
	assert_eq!(ports[0].protocol, "udp");
	assert!(ports[0].host_port.is_none());
}

#[test]
fn range_expansion() {
	let ports = parse_ports(&[short("8000-8002:8000-8002")]).unwrap();
	assert_eq!(ports.len(), 3);
	assert_eq!(ports[0].container_port, 8000);
	assert_eq!(ports[1].container_port, 8001);
	assert_eq!(ports[2].container_port, 8002);
}

#[test]
fn ipv6_host_ip() {
	let ports = parse_ports(&[short("[::1]:8080:80")]).unwrap();
	assert_eq!(ports[0].host_ip, "::1");
	assert_eq!(ports[0].host_port, Some(8080));
	assert_eq!(ports[0].container_port, 80);
}

#[test]
fn random_host_port_via_zero() {
	let ports = parse_ports(&[short("0:80")]).unwrap();
	assert_eq!(ports[0].container_port, 80);
	assert_eq!(ports[0].host_port, Some(0));
}

#[test]
fn sctp_protocol() {
	let ports = parse_ports(&[short("80:80/sctp")]).unwrap();
	assert_eq!(ports[0].protocol, "sctp");
}

#[test]
fn range_with_udp() {
	let ports = parse_ports(&[short("5000-5010:5000-5010/udp")]).unwrap();
	assert_eq!(ports.len(), 11);
	assert!(ports.iter().all(|p| p.protocol == "udp"));
}

#[test]
fn range_too_large_rejected() {
	assert!(parse_ports(&[short("1-1025")]).is_err());
	assert!(parse_ports(&[short("1-65535")]).is_err());
}

#[test]
fn range_at_limit_accepted() {
	let ports = parse_ports(&[short("1-1024")]).unwrap();
	assert_eq!(ports.len(), 1024);
}

#[test]
fn single_host_port_expands_over_container_range() {
	// Docker-compose semantics: 8080:80-82 → 8080→80, 8081→81, 8082→82
	let ports = parse_ports(&[short("8080:80-82")]).unwrap();
	assert_eq!(ports.len(), 3);
	assert_eq!(ports[0].host_port, Some(8080));
	assert_eq!(ports[0].container_port, 80);
	assert_eq!(ports[1].host_port, Some(8081));
	assert_eq!(ports[1].container_port, 81);
	assert_eq!(ports[2].host_port, Some(8082));
	assert_eq!(ports[2].container_port, 82);
}

#[test]
fn long_form_invalid_published_string_rejected() {
	use podup::compose::types::StringOrU16;
	let mapping = podup::compose::types::PortMapping::Long {
		target: 80,
		published: Some(StringOrU16::String("invalid".to_string())),
		protocol: None,
		host_ip: None,
		mode: None,
		app_protocol: None,
		name: None,
	};
	assert!(parse_ports(&[mapping]).is_err());
}

/// The compose specification lists `127.0.0.1::80` among the valid forms:
/// the port is published on that address, on a host port the runtime picks.
#[test]
fn an_ip_with_no_host_port() {
	let ports = parse_ports(&[short("127.0.0.1::80")]).unwrap();
	assert_eq!(ports.len(), 1);
	assert_eq!(ports[0].container_port, 80);
	assert_eq!(ports[0].host_ip, "127.0.0.1");
	assert!(ports[0].host_port.is_none());
}

/// The compose specification lists `8000-9000:80` among the valid forms: a
/// range of host ports published on a single container port.
#[test]
fn a_host_range_with_a_single_container_port() {
	let ports = parse_ports(&[short("8000-9000:80")]).unwrap();
	assert_eq!(ports.len(), 1001);
	assert_eq!(ports[0].host_port, Some(8000));
	assert_eq!(ports[0].container_port, 80);
	assert_eq!(ports[1000].host_port, Some(9000));
	assert_eq!(ports[1000].container_port, 80);
}

/// The same form with an address: every port of the range is published on
/// that address.
#[test]
fn a_host_range_with_an_ip_and_a_single_container_port() {
	let ports = parse_ports(&[short("127.0.0.1:8000-8002:80")]).unwrap();
	assert_eq!(ports.len(), 3);
	assert_eq!(ports[0].host_port, Some(8000));
	assert_eq!(ports[0].container_port, 80);
	assert_eq!(ports[0].host_ip, "127.0.0.1");
	assert_eq!(ports[2].host_port, Some(8002));
	assert_eq!(ports[2].container_port, 80);
}

/// The long form accepts a range of published ports.
#[test]
fn a_published_range_in_the_long_form() {
	use podup::compose::types::StringOrU16;
	let mapping = podup::compose::types::PortMapping::Long {
		target: 80,
		published: Some(StringOrU16::String("8080-8081".to_string())),
		protocol: None,
		host_ip: None,
		mode: None,
		app_protocol: None,
		name: None,
	};
	let ports = parse_ports(&[mapping]).unwrap();
	assert_eq!(ports.len(), 2);
	assert_eq!(ports[0].host_port, Some(8080));
	assert_eq!(ports[0].container_port, 80);
	assert_eq!(ports[1].host_port, Some(8081));
	assert_eq!(ports[1].container_port, 80);
}
