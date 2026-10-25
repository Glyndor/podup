//! The default that keeps two bridge networks from seeing each other, exported
//! the way the live engine creates the network.

use super::unit_named;
use crate::parse_str;
use crate::quadlet::generate_at;

fn network_contents(networks: &str) -> String {
	let yaml = format!(
		"services:\n  web:\n    image: nginx\n    networks: [front]\nnetworks:\n  front:{networks}\n"
	);
	let file = parse_str(&yaml).unwrap();
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));
	unit_named(&out, "proj-front.network").contents.clone()
}

#[test]
fn a_default_bridge_network_is_isolated() {
	for declaration in [" {}", "\n    driver: bridge"] {
		let unit = network_contents(declaration);
		assert!(
			unit.contains("Options=isolate=true"),
			"bridge network not isolated ({declaration:?}):\n{unit}"
		);
	}
}

#[test]
fn the_file_can_turn_isolation_off() {
	let unit = network_contents("\n    driver_opts:\n      isolate: \"false\"");
	assert!(
		unit.contains("Options=isolate=false") && !unit.contains("isolate=true"),
		"explicit isolate=false was not kept:\n{unit}"
	);
}

#[test]
fn only_a_bridge_network_gets_the_default() {
	let unit = network_contents("\n    driver: macvlan");
	assert!(
		!unit.contains("isolate"),
		"a non bridge network got the bridge default:\n{unit}"
	);
}

#[test]
fn options_still_come_before_the_labels() {
	let unit =
		network_contents("\n    driver_opts:\n      mtu: \"1400\"\n    labels:\n      tier: front");
	let options = unit.find("Options=mtu=1400").expect("mtu option");
	let isolate = unit.find("Options=isolate=true").expect("isolation option");
	let label = unit.find("Label=tier=front").expect("label");
	assert!(
		options < label && isolate < label,
		"options moved after the labels:\n{unit}"
	);
}
