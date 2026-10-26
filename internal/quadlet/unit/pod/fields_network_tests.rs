//! Static pod addresses require exactly one network attachment.

use super::fields_test_support::{render, unit};

fn assert_multiple_networks(field: &str, key: &str, flag: &str, address: &str) {
	for attachment in ["      back: {}\n", ""] {
		let yaml = format!("x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    networks:\n      front:\n        {field}: {address:?}\n{attachment}networks:\n  front:\n    enable_ipv6: true\n    ipam:\n      config:\n        - subnet: 10.5.0.0/24\n        - subnet: 2001:db8::/64\n  back: {{}}\n");
		let out = render(&yaml);
		let pod = &unit(&out, "demo.pod").contents;
		assert!(pod.contains("Network=demo-front.network\n"), "{pod}");
		assert!(pod.contains("Network=demo-back.network\n"), "{pod}");
		assert!(
			!pod.contains(flag),
			"unscoped address on two networks: {pod}"
		);
		assert!(unit(&out, "demo-web.container")
			.contents
			.contains(&format!("{key}={address}\n")));
		assert!(
			out.warnings
				.iter()
				.any(|warning| warning.contains(field)
					&& warning.contains("stays on each container")),
			"{:?}",
			out.warnings
		);
	}
}

#[test]
fn static_ipv4_stays_on_container_with_two_pod_networks() {
	assert_multiple_networks("ipv4_address", "IP", "--ip=", "10.5.0.7");
}

#[test]
fn static_ipv6_stays_on_container_with_two_pod_networks() {
	assert_multiple_networks("ipv6_address", "IP6", "--ip6=", "2001:db8::7");
}
