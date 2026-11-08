use super::*;

// --- render_publish_port --------------------------------------------------

#[test]
fn render_publish_port_full_and_partial_forms() {
	let port = |host_ip: &str, host_port: Option<u16>, cp: u16, proto: &str| ParsedPort {
		container_port: cp,
		protocol: proto.to_string(),
		host_ip: host_ip.to_string(),
		host_port,
	};
	// ip + host port + container port, default tcp omits the protocol suffix.
	assert_eq!(
		render_publish_port(&port("127.0.0.1", Some(8080), 80, "tcp")),
		"127.0.0.1:8080:80"
	);
	// No ip, no host port (runtime-assigned) → bare container port.
	assert_eq!(render_publish_port(&port("", None, 80, "tcp")), "80");
	// A non-tcp protocol is appended; a 0 host port is treated as "let Podman pick".
	assert_eq!(render_publish_port(&port("", Some(0), 53, "udp")), "53/udp");
}

#[test]
fn render_publish_port_brackets_an_ipv6_host_address() {
	let port = |host_ip: &str| ParsedPort {
		container_port: 80,
		protocol: "tcp".to_string(),
		host_ip: host_ip.to_string(),
		host_port: Some(8080),
	};
	// An IPv6 address carries colons, so Podman needs the brackets to tell
	// the address from the port.
	assert_eq!(render_publish_port(&port("::1")), "[::1]:8080:80");
	// An address that already carries the brackets is left alone.
	assert_eq!(render_publish_port(&port("[::1]")), "[::1]:8080:80");
	// An IPv4 address is written as it stands.
	assert_eq!(render_publish_port(&port("127.0.0.1")), "127.0.0.1:8080:80");
}
