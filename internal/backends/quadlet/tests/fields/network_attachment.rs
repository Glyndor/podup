//! What a service sets on one of its networks, in the exported `Network=` line:
//! the interface name, the driver options and the order the lines come in.
//!
//! Podman's CLI reads `--network <name>:<options>` and hands every option it
//! does not know to the network backend, so a driver option travels as a plain
//! `key=value` next to `alias=`, `ip=` and `interface_name=`. The order of the
//! `--network` flags is the order the networks are connected in, and the first
//! holds `eth0`.

use std::process::Command;

use crate::backends::quadlet::tests::generator::generator;
use crate::parse_str;
use crate::quadlet::generate_at;
use crate::quadlet::tests::unit_named;

fn web_unit(networks: &str, top: &str) -> (String, Vec<String>) {
	let yaml =
		format!("services:\n  web:\n    image: x\n    networks:\n{networks}networks:\n{top}");
	let file = parse_str(&yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	(
		unit_named(&out, "p-web.container").contents.clone(),
		out.warnings.clone(),
	)
}

fn network_lines(unit: &str) -> Vec<&str> {
	unit.lines().filter(|l| l.starts_with("Network=")).collect()
}

const TWO: &str = "  front: {}\n  back: {}\n";

#[test]
fn interface_name_and_driver_options_go_on_the_network_line() {
	let (unit, _) = web_unit(
		"      front:\n        interface_name: eth7\n        driver_opts:\n          host_interface_name: vethweb\n          mtu: \"1400\"\n",
		"  front: {}\n",
	);
	assert_eq!(
		network_lines(&unit),
		vec!["Network=p-front.network:alias=web,interface_name=eth7,host_interface_name=vethweb,mtu=1400"],
		"{unit}"
	);
}

#[test]
fn driver_options_are_written_in_name_order() {
	let (unit, _) = web_unit(
		"      front:\n        driver_opts:\n          zeta: \"1\"\n          alpha: \"2\"\n",
		"  front: {}\n",
	);
	assert_eq!(
		network_lines(&unit),
		vec!["Network=p-front.network:alias=web,alpha=2,zeta=1"]
	);
}

#[test]
fn an_option_podman_reads_as_its_own_is_not_written_as_a_driver_option() {
	let (unit, _) = web_unit(
		"      front:\n        driver_opts:\n          alias: sneaky\n          ip: 10.0.0.9\n          mac: aa:bb:cc:dd:ee:ff\n          mtu: \"1400\"\n",
		"  front: {}\n",
	);
	assert_eq!(
		network_lines(&unit),
		vec!["Network=p-front.network:alias=web,mtu=1400"],
		"{unit}"
	);
}

#[test]
fn a_value_or_name_that_could_split_the_list_is_left_out() {
	let (unit, _) = web_unit(
		"      front:\n        interface_name: \"eth0,ip=1.2.3.4\"\n        driver_opts:\n          a: \"1,ip=10.0.0.9\"\n          \"b c\": \"2\"\n          d: \"x y\"\n          ok: \"1\"\n",
		"  front: {}\n",
	);
	assert_eq!(
		network_lines(&unit),
		vec!["Network=p-front.network:alias=web,ok=1"],
		"{unit}"
	);
	assert!(!unit.contains("ip=1.2.3.4"), "{unit}");
}

#[test]
fn the_highest_priority_is_written_first_and_equal_ones_by_name() {
	let (unit, _) = web_unit(
		"      front:\n        priority: 10\n      back:\n        priority: 1000\n      mid:\n        priority: 10\n",
		"  front: {}\n  back: {}\n  mid: {}\n",
	);
	let names: Vec<&str> = network_lines(&unit)
		.into_iter()
		.map(|l| {
			l.trim_start_matches("Network=p-")
				.split('.')
				.next()
				.unwrap()
		})
		.collect();
	assert_eq!(names, vec!["back", "front", "mid"], "{unit}");
}

#[test]
fn without_a_priority_the_networks_keep_their_declared_order() {
	let (unit, _) = web_unit("      front: {}\n      back: {}\n", TWO);
	let names: Vec<&str> = network_lines(&unit)
		.into_iter()
		.map(|l| {
			l.trim_start_matches("Network=p-")
				.split('.')
				.next()
				.unwrap()
		})
		.collect();
	assert_eq!(names, vec!["front", "back"], "{unit}");
}

#[test]
fn a_priority_of_zero_alone_changes_nothing() {
	let (unit, _) = web_unit("      front:\n        priority: 0\n      back: {}\n", TWO);
	let names: Vec<&str> = network_lines(&unit)
		.into_iter()
		.map(|l| {
			l.trim_start_matches("Network=p-")
				.split('.')
				.next()
				.unwrap()
		})
		.collect();
	assert_eq!(names, vec!["front", "back"], "{unit}");
}

#[test]
fn link_local_ips_are_not_written_as_addresses() {
	let (unit, _) = web_unit(
		"      front:\n        link_local_ips: [169.254.5.5]\n",
		"  front: {}\n",
	);
	assert_eq!(
		network_lines(&unit),
		vec!["Network=p-front.network:alias=web"]
	);
	assert!(!unit.contains("169.254"), "{unit}");
	assert!(!unit.contains("IP="), "{unit}");
}

#[test]
fn inside_a_pod_the_per_network_options_are_reported_not_written() {
	let yaml = "x-podman-pod: true\nservices:\n  web:\n    image: x\n    networks:\n      front:\n        interface_name: eth7\n        priority: 5\n        driver_opts:\n          mtu: \"1400\"\nnetworks:\n  front: {}\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let joined = out.warnings.join("\n");
	for field in ["interface_name", "driver_opts", "priority"] {
		assert!(
			joined.contains(&format!("networks.*.{field} is not exported inside a pod")),
			"{field}: {joined}"
		);
	}
	for unit in &out.units {
		assert!(!unit.contents.contains("eth7"), "{}", unit.filename);
	}
}

/// Podman's generator reads the written line back into `--network` flags in the
/// same order and with the same options.
#[test]
fn the_generator_passes_the_options_and_the_order_to_podman() {
	let Some(generator) = generator() else {
		return;
	};
	let (unit, _) = web_unit(
		"      front:\n        priority: 1\n        interface_name: eth7\n        driver_opts:\n          host_interface_name: vethweb\n      back:\n        priority: 9\n",
		TWO,
	);
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("p-web.container"), &unit).unwrap();
	for net in ["front", "back"] {
		std::fs::write(dir.path().join(format!("p-{net}.network")), "[Network]\n").unwrap();
	}
	let run = Command::new(&generator)
		.args(["-dryrun", "-user"])
		.env("QUADLET_UNIT_DIRS", dir.path())
		.output()
		.unwrap();
	let stdout = String::from_utf8_lossy(&run.stdout);
	let exec = stdout
		.lines()
		.find(|l| l.starts_with("ExecStart=") && l.contains("--network"))
		.unwrap_or_else(|| {
			panic!(
				"no container ExecStart: {stdout}\n{}",
				String::from_utf8_lossy(&run.stderr)
			)
		});
	let back = exec.find("--network systemd-p-back:alias=web").expect(exec);
	let front = exec
		.find("--network systemd-p-front:alias=web,interface_name=eth7,host_interface_name=vethweb")
		.expect(exec);
	assert!(back < front, "higher priority first: {exec}");
}
