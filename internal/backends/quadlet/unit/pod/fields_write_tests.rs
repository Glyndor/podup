//! Pod-unit rendering tests for fields shared by every member container.

use super::fields_test_support::{pod_contents, render, two_services, two_services_on_front, unit};

#[test]
fn hostname_stays_on_each_container() {
	let out = render(&two_services("    hostname: shared-host\n"));
	assert!(!unit(&out, "demo.pod").contents.contains("--hostname="));
	for name in ["demo-web.container", "demo-db.container"] {
		let container = &unit(&out, name).contents;
		assert!(container.contains("HostName=shared-host\n"), "{container}");
	}
}

#[test]
fn pod_writes_dns() {
	let pod = pod_contents(&two_services("    dns:\n      - 1.1.1.1\n"));
	assert!(pod.contains("DNS=1.1.1.1\n"), "missing DNS flag in:\n{pod}");
}

#[test]
fn pod_writes_dns_option() {
	let pod = pod_contents(&two_services("    dns_opt:\n      - ndots:1\n"));
	assert!(
		pod.contains("DNSOption=ndots:1\n"),
		"missing DNS option flag in:\n{pod}"
	);
}

#[test]
fn pod_writes_dns_search() {
	let pod = pod_contents(&two_services("    dns_search:\n      - example.com\n"));
	assert!(
		pod.contains("DNSSearch=example.com\n"),
		"missing DNS search flag in:\n{pod}"
	);
}

#[test]
fn pod_writes_extra_hosts() {
	let pod = pod_contents(&two_services("    extra_hosts:\n      - \"db:10.0.0.2\"\n"));
	assert!(
		pod.contains("AddHost=db:10.0.0.2\n"),
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
		pod.contains("UserNS=keep-id\n"),
		"missing user namespace flag in:\n{pod}"
	);
}

#[test]
fn pod_writes_shm_size_in_bytes() {
	let pod = pod_contents(&two_services("    shm_size: 64m\n"));
	assert!(
		pod.contains("ShmSize=67108864\n"),
		"missing shared-memory flag in:\n{pod}"
	);
}

#[test]
fn pod_writes_static_ipv4() {
	let field = "    networks:\n      front:\n        ipv4_address: 10.5.0.7\n";
	let pod = pod_contents(&two_services_on_front(field));
	assert!(pod.contains("Network=demo-front.network\n"), "{pod}");
	assert!(
		pod.contains("IP=10.5.0.7\n"),
		"missing IPv4 flag in:\n{pod}"
	);
}

#[test]
fn pod_writes_static_ipv6() {
	let field = "    networks:\n      front:\n        ipv6_address: \"2001:db8::7\"\n";
	let pod = pod_contents(&two_services_on_front(field));
	assert!(pod.contains("Network=demo-front.network\n"), "{pod}");
	assert!(
		pod.contains("IP6=2001:db8::7\n"),
		"missing IPv6 flag in:\n{pod}"
	);
}

#[test]
fn pod_writes_network_alias() {
	let field = "    networks:\n      front:\n        aliases:\n          - shared-alias\n";
	let pod = pod_contents(&two_services_on_front(field));
	assert!(pod.contains("Network=demo-front.network\n"), "{pod}");
	assert!(
		pod.contains("NetworkAlias=shared-alias\n"),
		"missing network alias flag in:\n{pod}"
	);
}

#[test]
fn pod_keeps_service_labels_off_pod() {
	let pod = pod_contents(&two_services("    labels:\n      team: core\n"));
	assert!(
		!pod.contains("Label=team=core"),
		"pod took container metadata:\n{pod}"
	);
}
