//! Container-unit tests for fields moved to the project pod.

use super::pod_fields_test_support::{
	container_contents, pod_contents, two_services, two_services_on_front,
};

#[test]
fn pod_container_drops_hostname() {
	let container = container_contents(&two_services("    hostname: shared-host\n"));
	assert!(
		!container.contains("HostName="),
		"container repeated pod hostname in:\n{container}"
	);
}

#[test]
fn pod_container_drops_dns() {
	let container = container_contents(&two_services("    dns:\n      - 1.1.1.1\n"));
	assert!(
		!container.contains("DNS="),
		"container repeated pod DNS in:\n{container}"
	);
}

#[test]
fn pod_container_drops_dns_option() {
	let container = container_contents(&two_services("    dns_opt:\n      - ndots:1\n"));
	assert!(
		!container.contains("DNSOption="),
		"container repeated pod DNS option in:\n{container}"
	);
}

#[test]
fn pod_container_drops_dns_search() {
	let container = container_contents(&two_services("    dns_search:\n      - example.com\n"));
	assert!(
		!container.contains("DNSSearch="),
		"container repeated pod DNS search in:\n{container}"
	);
}

#[test]
fn pod_container_drops_extra_hosts() {
	let container =
		container_contents(&two_services("    extra_hosts:\n      - \"db:10.0.0.2\"\n"));
	assert!(
		!container.contains("--add-host=\"db:10.0.0.2\""),
		"container repeated pod host entry in:\n{container}"
	);
}

#[test]
fn pod_container_keeps_volume() {
	let yaml = "x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    volumes:\n      - data:/data\n  db:\n    image: postgres\n    volumes:\n      - data:/data\nvolumes:\n  data:\n";
	let container = container_contents(yaml);
	assert!(
		container.contains("Volume=demo-data.volume:/data"),
		"container lost its volume:\n{container}"
	);
}

#[test]
fn pod_container_drops_userns() {
	let container = container_contents(&two_services("    userns_mode: keep-id\n"));
	assert!(
		!container.contains("UserNS="),
		"container repeated pod user namespace in:\n{container}"
	);
}

#[test]
fn pod_container_drops_shm_size() {
	let container = container_contents(&two_services("    shm_size: 64m\n"));
	assert!(
		!container.contains("ShmSize="),
		"container repeated pod shared-memory size in:\n{container}"
	);
}

#[test]
fn pod_container_drops_static_ipv4() {
	let field = "    networks:\n      front:\n        ipv4_address: 10.5.0.7\n";
	let yaml = two_services_on_front(field);
	assert!(pod_contents(&yaml).contains("Network=demo-front.network\n"));
	let container = container_contents(&yaml);
	assert!(
		!container.lines().any(|line| line.starts_with("IP=")),
		"container repeated pod IPv4 address in:\n{container}"
	);
}

#[test]
fn pod_container_drops_static_ipv6() {
	let field = "    networks:\n      front:\n        ipv6_address: \"2001:db8::7\"\n";
	let yaml = two_services_on_front(field);
	assert!(pod_contents(&yaml).contains("Network=demo-front.network\n"));
	let container = container_contents(&yaml);
	assert!(
		!container.lines().any(|line| line.starts_with("IP6=")),
		"container repeated pod IPv6 address in:\n{container}"
	);
}

#[test]
fn pod_container_drops_network_alias() {
	let field = "    networks:\n      front:\n        aliases:\n          - shared-alias\n";
	let yaml = two_services_on_front(field);
	assert!(pod_contents(&yaml).contains("Network=demo-front.network\n"));
	let container = container_contents(&yaml);
	assert!(
		!container.contains("--network-alias=\"shared-alias\""),
		"container repeated pod network alias in:\n{container}"
	);
}

#[test]
fn pod_container_keeps_service_labels() {
	let field = "    labels:\n      team: core\n";
	for yaml in [
		two_services(field),
		format!("x-podman-pod: true\nservices:\n  web:\n    image: nginx\n{field}"),
	] {
		let out = super::pod_fields_test_support::render(&yaml);
		for container in out
			.units
			.iter()
			.filter(|unit| unit.filename.ends_with(".container"))
		{
			assert!(
				container.contents.contains("Label=team=core\n"),
				"{}",
				container.contents
			);
		}
		assert!(!pod_contents(&yaml).contains("--label=\"team=core\""));
	}
}

#[test]
fn anonymous_volume_stays_on_containers() {
	let yaml = "x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    volumes:\n      - data:/shared\n      - /private\n  db:\n    image: postgres\n    volumes:\n      - data:/shared\n      - /private\nvolumes:\n  data:\n";
	let pod = pod_contents(yaml);
	let container = container_contents(yaml);
	assert!(
		!pod.contains("Volume="),
		"pod mixed shared and anonymous volumes:\n{pod}"
	);
	assert!(
		container.contains("Volume=demo-data.volume:/shared")
			&& container.contains("Volume=/private"),
		"container did not keep its anonymous volume:\n{container}"
	);
}
