//! Pod-unit rendering tests for fields shared by every member container.

use super::pod_fields_test_support::{pod_contents, two_services, two_services_on_front};

#[test]
fn pod_writes_hostname() {
	let pod = pod_contents(&two_services("    hostname: shared-host\n"));
	assert!(
		pod.contains("PodmanArgs=--hostname=\"shared-host\""),
		"missing hostname flag in:\n{pod}"
	);
}

#[test]
fn pod_writes_dns() {
	let pod = pod_contents(&two_services("    dns:\n      - 1.1.1.1\n"));
	assert!(
		pod.contains("PodmanArgs=--dns=\"1.1.1.1\""),
		"missing DNS flag in:\n{pod}"
	);
}

#[test]
fn pod_writes_dns_option() {
	let pod = pod_contents(&two_services("    dns_opt:\n      - ndots:1\n"));
	assert!(
		pod.contains("PodmanArgs=--dns-option=\"ndots:1\""),
		"missing DNS option flag in:\n{pod}"
	);
}

#[test]
fn pod_writes_dns_search() {
	let pod = pod_contents(&two_services("    dns_search:\n      - example.com\n"));
	assert!(
		pod.contains("PodmanArgs=--dns-search=\"example.com\""),
		"missing DNS search flag in:\n{pod}"
	);
}

#[test]
fn pod_writes_extra_hosts() {
	let pod = pod_contents(&two_services("    extra_hosts:\n      - \"db:10.0.0.2\"\n"));
	assert!(
		pod.contains("PodmanArgs=--add-host=\"db:10.0.0.2\""),
		"missing add-host flag in:\n{pod}"
	);
}

#[test]
fn pod_keeps_volumes_off_infra() {
	let yaml = "x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    volumes:\n      - data:/data\n  db:\n    image: postgres\n    volumes:\n      - data:/data\nvolumes:\n  data:\n";
	let pod = pod_contents(yaml);
	assert!(
		!pod.contains("Volume="),
		"pod mounted a service volume:\n{pod}"
	);
}

#[test]
fn pod_writes_userns() {
	let pod = pod_contents(&two_services("    userns_mode: keep-id\n"));
	assert!(
		pod.contains("PodmanArgs=--userns=\"keep-id\""),
		"missing user namespace flag in:\n{pod}"
	);
}

#[test]
fn pod_writes_shm_size() {
	let pod = pod_contents(&two_services("    shm_size: 64m\n"));
	assert!(
		pod.contains("PodmanArgs=--shm-size=\"64m\""),
		"missing shared-memory flag in:\n{pod}"
	);
}

#[test]
fn pod_writes_static_ipv4() {
	let field = "    networks:\n      front:\n        ipv4_address: 10.5.0.7\n";
	let pod = pod_contents(&two_services_on_front(field));
	assert!(pod.contains("Network=demo-front.network\n"), "{pod}");
	assert!(
		pod.contains("PodmanArgs=--ip=\"10.5.0.7\""),
		"missing IPv4 flag in:\n{pod}"
	);
}

#[test]
fn pod_writes_static_ipv6() {
	let field = "    networks:\n      front:\n        ipv6_address: \"2001:db8::7\"\n";
	let pod = pod_contents(&two_services_on_front(field));
	assert!(pod.contains("Network=demo-front.network\n"), "{pod}");
	assert!(
		pod.contains("PodmanArgs=--ip6=\"2001:db8::7\""),
		"missing IPv6 flag in:\n{pod}"
	);
}

#[test]
fn pod_writes_network_alias() {
	let field = "    networks:\n      front:\n        aliases:\n          - shared-alias\n";
	let pod = pod_contents(&two_services_on_front(field));
	assert!(pod.contains("Network=demo-front.network\n"), "{pod}");
	assert!(
		pod.contains("PodmanArgs=--network-alias=\"shared-alias\""),
		"missing network alias flag in:\n{pod}"
	);
}

#[test]
fn pod_keeps_service_labels_off_pod() {
	let pod = pod_contents(&two_services("    labels:\n      team: core\n"));
	assert!(
		!pod.contains("PodmanArgs=--label=\"team=core\""),
		"pod took container metadata:\n{pod}"
	);
}
