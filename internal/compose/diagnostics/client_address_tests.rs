//! Tests for the published-port client-address warning (#1994).

use crate::parse_str;

use super::CLIENT_ADDRESS_NEEDLE;

fn warnings_for(yaml: &str) -> Vec<String> {
	let file = parse_str(yaml).unwrap();
	super::super::collect(&file)
		.into_iter()
		.filter(|m| m.contains(CLIENT_ADDRESS_NEEDLE))
		.collect()
}

fn service(extra: &str) -> String {
	format!("services:\n  web:\n    image: nginx\n{extra}")
}

fn with_mode(mode: &str) -> Vec<String> {
	warnings_for(&service(&format!(
		"    network_mode: {mode}\n    ports:\n      - \"8080:80\"\n"
	)))
}

#[test]
fn warns_for_a_published_port_on_the_project_network() {
	let w = warnings_for(&service("    ports:\n      - \"8080:80\"\n"));
	assert_eq!(w.len(), 1, "got: {w:?}");
	assert!(w[0].contains("service 'web'") && w[0].contains("use network_mode: pasta"));
}

#[test]
fn ports_without_a_host_port_are_published_too() {
	// A `ports:` entry with no host port is bound to a random host port, so
	// it goes through the same proxy as a fixed one.
	for ports in [
		"      - \"80\"\n",
		"      - \"80-81/udp\"\n",
		"      - target: 80\n",
		"      - target: 80\n        published: 8080\n",
	] {
		let w = warnings_for(&service(&format!("    ports:\n{ports}")));
		assert_eq!(w.len(), 1, "ports {ports:?}: {w:?}");
	}
}

#[test]
fn does_not_warn_without_ports() {
	assert!(warnings_for(&service("    expose:\n      - \"80\"\n")).is_empty());
	assert!(warnings_for(&service("")).is_empty());
}

#[test]
fn warns_for_an_explicit_bridge_mode() {
	for mode in ["bridge", "\"bridge:alias=x\""] {
		assert_eq!(with_mode(mode).len(), 1, "network_mode {mode}");
	}
}

#[test]
fn inside_a_pod_the_advice_is_to_leave_the_pod() {
	// `network_mode` is refused inside a pod, so advising pasta there would
	// send the reader to a configuration podup rejects.
	let w = warnings_for(
		"x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    ports:\n      - \"8080:80\"\n",
	);
	assert_eq!(w.len(), 1, "got: {w:?}");
	assert!(w[0].contains("outside the pod"), "got: {w:?}");
}

#[test]
fn slirp4netns_warns_unless_its_port_handler_keeps_the_source() {
	for mode in ["slirp4netns", "\"slirp4netns:allow_host_loopback=true\""] {
		let w = with_mode(mode);
		assert_eq!(w.len(), 1, "network_mode {mode}: {w:?}");
		assert!(w[0].contains("port_handler=slirp4netns"), "got: {w:?}");
	}
	assert!(with_mode("\"slirp4netns:port_handler=slirp4netns\"").is_empty());
	// Podman keeps the last handler given, so the order decides.
	assert_eq!(
		with_mode("\"slirp4netns:port_handler=slirp4netns,port_handler=rootlesskit\"").len(),
		1
	);
	assert!(
		with_mode("\"slirp4netns:port_handler=rootlesskit,port_handler=slirp4netns\"").is_empty()
	);
}

#[test]
fn does_not_warn_for_modes_without_the_proxy() {
	for mode in ["pasta", "\"pasta:-T,15432\"", "host", "none"] {
		assert!(with_mode(mode).is_empty(), "network_mode {mode}");
	}
}

#[test]
fn warns_once_per_service_with_several_ports() {
	let w = warnings_for(&service(
		"    ports:\n      - \"8080:80\"\n      - \"8443:443\"\n      - \"127.0.0.1:9000:9000\"\n",
	));
	assert_eq!(w.len(), 1, "got: {w:?}");
}

#[test]
fn the_gate_identifies_this_warning_and_not_the_port_exposure_one() {
	let w = warnings_for(&service("    ports:\n      - \"8080:80\"\n"));
	assert!(super::super::is_client_address_warning(&w[0]));
	assert!(!super::super::is_port_exposure_warning(&w[0]));
}
