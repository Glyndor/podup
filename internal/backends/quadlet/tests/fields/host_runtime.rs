//! Keys that pick what the container runs on: its GPUs, the image platform and
//! the OCI runtime. Each reaches the unit the way `up` applies it.

use crate::parse_str;
use crate::quadlet::generate_at;
use crate::quadlet::tests::unit_named;

/// Generate service `s` of project `p` and return its unit and the warnings.
fn render(service_body: &str) -> (String, String) {
	let yaml = format!("services:\n  s:\n    image: app:1.0\n{service_body}");
	let file = parse_str(&yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let unit = unit_named(&out, "p-s.container").contents.clone();
	(unit, out.warnings.join("\n"))
}

fn lines_starting<'a>(unit: &'a str, prefix: &str) -> Vec<&'a str> {
	unit.lines().filter(|l| l.starts_with(prefix)).collect()
}

#[test]
fn gpus_all_adds_every_nvidia_gpu() {
	let (unit, warnings) = render("    gpus: all\n");
	assert_eq!(
		lines_starting(&unit, "AddDevice="),
		["AddDevice=nvidia.com/gpu=all"],
		"{unit}"
	);
	assert!(!warnings.contains("gpus"), "{warnings}");
}

#[test]
fn a_gpu_count_adds_that_many_devices() {
	let (unit, _) = render("    gpus: 2\n");
	assert_eq!(
		lines_starting(&unit, "AddDevice="),
		["AddDevice=nvidia.com/gpu=0", "AddDevice=nvidia.com/gpu=1"],
		"{unit}"
	);
}

#[test]
fn a_deploy_gpu_reservation_adds_its_device_ids() {
	let (unit, _) = render(
		"    deploy:\n      resources:\n        reservations:\n          devices:\n            - capabilities: [gpu]\n              device_ids: [\"3\"]\n",
	);
	assert_eq!(
		lines_starting(&unit, "AddDevice="),
		["AddDevice=nvidia.com/gpu=3"],
		"{unit}"
	);
}

#[test]
fn a_reservation_for_another_driver_adds_no_device() {
	let (unit, _) = render(
		"    deploy:\n      resources:\n        reservations:\n          devices:\n            - driver: amd\n              capabilities: [gpu]\n",
	);
	assert!(lines_starting(&unit, "AddDevice=").is_empty(), "{unit}");
}

#[test]
fn a_gpu_already_named_in_devices_is_added_once() {
	let (unit, _) = render("    devices: [nvidia.com/gpu=all]\n    gpus: all\n");
	assert_eq!(
		lines_starting(&unit, "AddDevice="),
		["AddDevice=nvidia.com/gpu=all"],
		"{unit}"
	);
}

#[test]
fn platform_reaches_podman_run_with_its_variant() {
	let (unit, warnings) = render("    platform: linux/arm64/v8\n");
	let args = lines_starting(&unit, "PodmanArgs=--platform=");
	assert_eq!(args.len(), 1, "{unit}");
	assert!(args[0].contains("linux/arm64/v8"), "{unit}");
	assert!(!warnings.contains("platform"), "{warnings}");
}

#[test]
fn runtime_is_a_global_podman_option() {
	// `--runtime` belongs before `run`; as a `run` flag Podman refuses it.
	let (unit, warnings) = render("    runtime: runc\n");
	let args = lines_starting(&unit, "GlobalArgs=--runtime=");
	assert_eq!(args.len(), 1, "{unit}");
	assert!(args[0].contains("runc"), "{unit}");
	assert!(!unit.contains("PodmanArgs=--runtime"), "{unit}");
	assert!(!warnings.contains("runtime"), "{warnings}");
}

#[test]
fn none_of_the_keys_adds_anything_when_absent() {
	let (unit, _) = render("");
	assert!(lines_starting(&unit, "AddDevice=").is_empty(), "{unit}");
	assert!(lines_starting(&unit, "GlobalArgs=").is_empty(), "{unit}");
	assert!(!unit.contains("--platform"), "{unit}");
}
