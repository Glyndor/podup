//! Names a compose file gives that Quadlet would read as something else.
//!
//! `Image=`, `Network=` and `Volume=` are not plain strings to Quadlet. A value
//! that starts with `-` lands in the position of the image in the `podman run`
//! line and is read as a flag; a network named `x.network` or `x.container`
//! asks for a unit of that name; `container:other` and `ns:/path` join another
//! namespace; a volume named `x.volume` asks for a unit. `up` reads each of
//! these as what it says (an image that cannot be pulled, a network or volume
//! that does not exist), so the unit has to refuse them too instead of writing
//! a line that means something the compose file never said.

use crate::parse_str;
use crate::quadlet::validate_for_quadlet;

fn refusal(yaml: &str) -> String {
	validate_for_quadlet(&parse_str(yaml).unwrap())
		.expect_err("validation must refuse it")
		.to_string()
}

fn network_yaml(name: &str) -> String {
	format!(
		"services:\n  web:\n    image: nginx\n    networks: [n]\nnetworks:\n  n:\n    external: true\n    name: \"{name}\"\n"
	)
}

fn volume_yaml(name: &str) -> String {
	format!(
		"services:\n  web:\n    image: nginx\n    volumes: [\"v:/data\"]\nvolumes:\n  v:\n    external: true\n    name: \"{name}\"\n"
	)
}

#[test]
fn an_image_that_starts_with_a_dash_is_refused() {
	let msg = refusal("services:\n  web:\n    image: \"--cidfile=/tmp/x\"\n");
	assert!(
		msg.contains("web") && msg.contains("--cidfile=/tmp/x"),
		"{msg}"
	);
}

#[test]
fn an_image_with_a_dash_inside_is_accepted() {
	let yaml = "services:\n  web:\n    image: ghcr.io/org/my-app:1.0-rc1\n";
	validate_for_quadlet(&parse_str(yaml).unwrap()).expect("a dash inside is an ordinary name");
}

#[test]
fn an_external_network_cannot_be_named_as_a_unit_or_a_namespace() {
	for name in [
		"prod-net.network",
		"other.container",
		"container:other",
		"ns:/proc/1/ns/net",
		"pasta:--map-host-loopback",
	] {
		let msg = refusal(&network_yaml(name));
		assert!(
			msg.contains(name) && msg.contains("network"),
			"{name}: {msg}"
		);
	}
}

#[test]
fn an_external_network_with_an_ordinary_name_is_accepted() {
	for name in ["prod-net", "prod.net", "net_1", "ext.pod", "ext.volume"] {
		validate_for_quadlet(&parse_str(&network_yaml(name)).unwrap())
			.unwrap_or_else(|e| panic!("{name} was refused: {e}"));
	}
}

#[test]
fn an_external_volume_cannot_be_named_as_a_unit() {
	for name in ["ext.volume", "a:b"] {
		let msg = refusal(&volume_yaml(name));
		assert!(
			msg.contains(name) && msg.contains("volume"),
			"{name}: {msg}"
		);
	}
}

#[test]
fn an_external_volume_with_an_ordinary_name_is_accepted() {
	for name in ["data", "data.v1", "ext.container", "ext.network"] {
		validate_for_quadlet(&parse_str(&volume_yaml(name)).unwrap())
			.unwrap_or_else(|e| panic!("{name} was refused: {e}"));
	}
}

#[test]
fn a_name_nothing_uses_is_not_checked() {
	let yaml = "services:\n  web:\n    image: nginx\nnetworks:\n  n:\n    external: true\n    name: \"x.network\"\n";
	validate_for_quadlet(&parse_str(yaml).unwrap())
		.expect("an unused network never reaches a unit");
}
