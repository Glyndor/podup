use super::*;

#[test]
fn warns_on_network_enable_ipv4() {
	let msgs = diagnostics_for(
		"services:\n  web:\n    image: nginx\nnetworks:\n  net:\n    enable_ipv4: false\n",
	);
	assert!(msgs.iter().any(|m| m.contains("enable_ipv4")));
}

#[test]
fn warns_on_service_network_gw_priority() {
	let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    networks:\n      net:\n        gw_priority: 10\nnetworks:\n  net:\n",
		);
	assert!(
		msgs.iter().any(|m| m.contains("gw_priority")),
		"got: {msgs:?}"
	);
}

#[test]
fn warns_on_long_port_mode() {
	let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    ports:\n      - target: 80\n        published: 8080\n        mode: host\n",
		);
	assert!(
		msgs.iter().any(|m| m.contains("port mode 'host'")),
		"got: {msgs:?}"
	);
}

#[test]
fn does_not_warn_on_interface_name() {
	// interface_name IS forwarded to Podman (PerNetworkOptions.interface_name),
	// so it must not produce a "not forwarded / ignored" warning.
	let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    networks:\n      net:\n        interface_name: eth9\nnetworks:\n  net:\n",
		);
	assert!(
		!msgs.iter().any(|m| m.contains("interface_name")),
		"interface_name should not be reported as ignored; got: {msgs:?}"
	);
}

#[test]
fn warns_on_ipam_aux_addresses() {
	let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\nnetworks:\n  net:\n    ipam:\n      config:\n        - subnet: 10.0.0.0/24\n          aux_addresses:\n            host1: 10.0.0.5\n",
		);
	assert!(
		msgs.iter().any(|m| m.contains("aux_addresses")),
		"got: {msgs:?}"
	);
}
