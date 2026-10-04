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

#[test]
fn warns_for_a_published_port_on_the_project_network() {
	let w = warnings_for(&service("    ports:\n      - \"8080:80\"\n"));
	assert_eq!(w.len(), 1, "got: {w:?}");
	assert!(w[0].contains("service 'web'") && w[0].contains("network_mode: pasta"));
}

#[test]
fn warns_for_a_long_form_published_port() {
	let w = warnings_for(&service(
		"    ports:\n      - target: 80\n        published: 8080\n",
	));
	assert_eq!(w.len(), 1, "got: {w:?}");
}

#[test]
fn warns_for_an_explicit_bridge_mode() {
	for mode in ["bridge", "\"bridge:alias=x\""] {
		let w = warnings_for(&service(&format!(
			"    network_mode: {mode}\n    ports:\n      - \"8080:80\"\n"
		)));
		assert_eq!(w.len(), 1, "network_mode {mode}: {w:?}");
	}
}

#[test]
fn warns_inside_a_pod() {
	let w = warnings_for(
		"x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    ports:\n      - \"8080:80\"\n",
	);
	assert_eq!(w.len(), 1, "got: {w:?}");
}

#[test]
fn warns_once_per_service_with_several_ports() {
	let w = warnings_for(&service(
		"    ports:\n      - \"8080:80\"\n      - \"8443:443\"\n      - \"127.0.0.1:9000:9000\"\n",
	));
	assert_eq!(w.len(), 1, "got: {w:?}");
}

#[test]
fn does_not_warn_without_a_host_port() {
	assert!(warnings_for(&service("    ports:\n      - \"80\"\n")).is_empty());
	assert!(warnings_for(&service("    ports:\n      - target: 80\n")).is_empty());
	assert!(warnings_for(&service("")).is_empty());
}

#[test]
fn does_not_warn_when_the_mode_keeps_the_source_or_has_no_bridge() {
	for mode in [
		"pasta",
		"\"pasta:-T,15432\"",
		"slirp4netns",
		"\"slirp4netns:allow_host_loopback=true\"",
		"host",
		"none",
	] {
		let w = warnings_for(&service(&format!(
			"    network_mode: {mode}\n    ports:\n      - \"8080:80\"\n"
		)));
		assert!(w.is_empty(), "network_mode {mode}: {w:?}");
	}
}

#[test]
fn the_suppression_gate_covers_this_warning() {
	let w = warnings_for(&service("    ports:\n      - \"8080:80\"\n"));
	assert!(super::super::is_port_exposure_warning(&w[0]));
}
