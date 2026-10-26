use super::*;

#[test]
fn format_ports_defaults_missing_host_ip_to_all_interfaces() {
	let p = ContainerPort {
		host_ip: None,
		host_port: Some(8080),
		container_port: 80,
		protocol: Some("tcp".into()),
		..Default::default()
	};
	assert_eq!(
		format_ports(std::slice::from_ref(&p)),
		"0.0.0.0:8080->80/tcp"
	);
}

#[test]
fn format_ports_keeps_explicit_host_ip() {
	let p = ContainerPort {
		host_ip: Some("127.0.0.1".into()),
		host_port: Some(5432),
		container_port: 5432,
		..Default::default()
	};
	assert_eq!(
		format_ports(std::slice::from_ref(&p)),
		"127.0.0.1:5432->5432"
	);
}

#[test]
fn format_ports_expands_a_collapsed_range() {
	// libpod collapses 51251-51253->8080-8082 into one record with range=3;
	// the full range must be rendered, not just the first mapping.
	let p = ContainerPort {
		host_ip: None,
		host_port: Some(51251),
		container_port: 8080,
		protocol: Some("tcp".into()),
		range: Some(3),
	};
	assert_eq!(
		format_ports(std::slice::from_ref(&p)),
		"0.0.0.0:51251-51253->8080-8082/tcp"
	);
}

#[test]
fn format_port_record_does_not_overflow_u16_on_pathological_range() {
	// `host_port`/`container_port`/`range` come straight from libpod's JSON:
	// untrusted input. host_port=65535 with a range of 2 needs
	// `host_port + (range - 1)` = 65536, which does not fit in a u16: it
	// wraps to 0 in release and panics under overflow-checks (this test runs
	// in a debug build, so a regression here panics rather than silently
	// passing). The rendered end-of-range must show the real, wider number
	// instead of wrapping.
	let p = ContainerPort {
		host_ip: None,
		host_port: Some(65535),
		container_port: 65535,
		protocol: Some("tcp".into()),
		range: Some(2),
	};
	let rendered = format_ports(std::slice::from_ref(&p));
	assert_eq!(rendered, "0.0.0.0:65535-65536->65535-65536/tcp");
}

#[test]
fn publishers_expand_each_port_in_a_range() {
	let p = ContainerPort {
		host_ip: Some("0.0.0.0".into()),
		host_port: Some(51251),
		container_port: 8080,
		protocol: Some("tcp".into()),
		range: Some(3),
	};
	let pubs = publishers(std::slice::from_ref(&p));
	assert_eq!(pubs.len(), 3);
	assert_eq!(pubs[0]["TargetPort"], 8080);
	assert_eq!(pubs[0]["PublishedPort"], 51251);
	assert_eq!(pubs[2]["TargetPort"], 8082);
	assert_eq!(pubs[2]["PublishedPort"], 51253);
	assert_eq!(pubs[1]["Protocol"], "tcp");
}

#[test]
fn publishers_does_not_overflow_u16_on_pathological_range() {
	// Same untrusted-input hazard as `format_port_record`: expanding
	// container_port=65535 over a range of 2 must not wrap the last entry's
	// TargetPort/PublishedPort to 0.
	let p = ContainerPort {
		host_ip: Some("0.0.0.0".into()),
		host_port: Some(65535),
		container_port: 65535,
		protocol: Some("tcp".into()),
		range: Some(2),
	};
	let pubs = publishers(std::slice::from_ref(&p));
	assert_eq!(pubs.len(), 2);
	assert_eq!(pubs[1]["TargetPort"], 65536);
	assert_eq!(pubs[1]["PublishedPort"], 65536);
}
