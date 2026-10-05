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
fn inside_a_pod_with_no_network_mode_the_advice_is_pasta_on_every_service() {
	// `network_mode` was previously refused on any service in a pod, so the
	// only pod-with-warning shape was "no mode on any service" and the
	// advice was to leave the pod. The pod validator still accepts that
	// shape (no service declares a mode, the pod runs on the project
	// networks), but the warning now points the user at the agreed-mode
	// configuration instead of "leave the pod".
	let w = warnings_for(
		"x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    ports:\n      - \"8080:80\"\n",
	);
	assert_eq!(w.len(), 1, "got: {w:?}");
	assert!(
		w[0].contains("set network_mode: pasta on every service of the pod"),
		"got: {w:?}"
	);
}

#[test]
fn inside_a_pod_with_agreed_pasta_there_is_no_warning() {
	// A pod where every service agrees on `pasta` (bare or with options)
	// keeps the client's address; the warning is silent.
	for mode in ["pasta", "\"pasta:-m,1400\""] {
		let yaml = format!(
			"x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    network_mode: {mode}\n    ports:\n      - \"8080:80\"\n  db:\n    image: postgres\n    network_mode: {mode}\n"
		);
		let w = warnings_for(&yaml);
		assert!(w.is_empty(), "mode {mode}: got: {w:?}");
	}
}

#[test]
fn inside_a_pod_with_agreed_slirp4netns_advises_port_handler() {
	let yaml = "x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    network_mode: slirp4netns\n    ports:\n      - \"8080:80\"\n  db:\n    image: postgres\n    network_mode: slirp4netns\n";
	let w = warnings_for(yaml);
	assert_eq!(w.len(), 1, "got: {w:?}");
	assert!(w[0].contains("port_handler=slirp4netns"), "got: {w:?}");
	assert!(w[0].contains("on every service"), "got: {w:?}");
}

#[test]
fn inside_a_pod_with_agreed_slirp4netns_and_port_handler_is_quiet() {
	let yaml = "x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    network_mode: \"slirp4netns:port_handler=slirp4netns\"\n    ports:\n      - \"8080:80\"\n  db:\n    image: postgres\n    network_mode: \"slirp4netns:port_handler=slirp4netns\"\n";
	assert!(
		warnings_for(yaml).is_empty(),
		"got: {:?}",
		warnings_for(yaml)
	);
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

#[test]
fn a_pod_with_a_partial_pasta_declaration_still_warns() {
	// One member on pasta and one without: the pod cannot run on pasta, so the
	// warning stays, whichever service is listed first.
	for (a, b) in [
		("", "    network_mode: pasta\n"),
		("    network_mode: pasta\n", ""),
	] {
		let yaml = format!(
			"x-podman-pod: true\nservices:\n  one:\n    image: nginx\n{a}    ports:\n      - \"8080:80\"\n  two:\n    image: nginx\n{b}"
		);
		let w = warnings_for(&yaml);
		assert_eq!(w.len(), 1, "{yaml}: {w:?}");
		assert!(w[0].contains("every service of the pod"), "{w:?}");
	}
}
