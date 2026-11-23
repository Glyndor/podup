//! `driver_opts` of a volume in the exported `.volume` unit.
//!
//! Podman 6.2.0's generator reads `Options=` on its own and turns it into
//! `--opt o=<value>`; it reads `Type=` only next to a `Device=` and drops the
//! whole unit otherwise (`key Type can't be used without Device`). The
//! tests that run the generator return early, with a `SKIP:` line, where it is
//! not installed or is older than the floor (see `generator.rs`).

use std::process::Command;

use crate::backends::quadlet::tests::generator::generator;
use crate::backends::quadlet::tests::unit_named;
use crate::parse_str;
use crate::quadlet::generate_at;

fn volume_unit(driver_opts: &str, driver: Option<&str>) -> String {
	let driver = driver
		.map(|d| format!("    driver: {d}\n"))
		.unwrap_or_default();
	let yaml = format!(
		"services:\n  web:\n    image: nginx\n    volumes:\n      - data:/data\nvolumes:\n  data:\n{driver}    driver_opts:\n{driver_opts}"
	);
	let file = parse_str(&yaml).unwrap();
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));
	unit_named(&out, "proj-data.volume").contents.clone()
}

/// What Podman's generator makes of the `.volume` unit: the `ExecStart=` of
/// the volume service, or `None` with the error it printed.
fn generated_exec(generator: &std::path::Path, unit: &str) -> Result<String, String> {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("proj-data.volume"), unit).unwrap();
	let run = Command::new(generator)
		.args(["-dryrun", "-user"])
		.env("QUADLET_UNIT_DIRS", dir.path())
		.output()
		.unwrap();
	let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
	stdout
		.lines()
		.find(|l| l.starts_with("ExecStart="))
		.map(str::to_string)
		.ok_or_else(|| String::from_utf8_lossy(&run.stderr).into_owned())
}

#[test]
fn mount_options_without_a_device_are_the_native_key() {
	let vol = volume_unit("      o: uid=1000,gid=1000\n", None);
	assert!(vol.contains("\nOptions=uid=1000,gid=1000\n"), "in:\n{vol}");
	assert!(!vol.contains("PodmanArgs="), "in:\n{vol}");
}

#[test]
fn a_type_without_a_device_keeps_going_through_opt() {
	// `Type=` here would make Podman drop the whole unit.
	let vol = volume_unit("      type: tmpfs\n      o: size=1g\n", None);
	assert!(!vol.contains("\nType="), "in:\n{vol}");
	assert!(vol.contains("\nOptions=size=1g\n"), "in:\n{vol}");
	assert!(
		vol.contains("PodmanArgs=--opt \"type\"=\"tmpfs\""),
		"in:\n{vol}"
	);
}

#[test]
fn options_next_to_a_device_stay_native() {
	let vol = volume_unit(
		"      device: /srv/data\n      type: bind\n      o: bind\n",
		None,
	);
	assert!(vol.contains("\nDevice=/srv/data\n"), "in:\n{vol}");
	assert!(vol.contains("\nType=bind\n"), "in:\n{vol}");
	assert!(vol.contains("\nOptions=bind\n"), "in:\n{vol}");
}

#[test]
fn a_mount_option_string_with_a_blank_at_its_end_is_not_trimmed_by_the_native_key() {
	let vol = volume_unit("      o: \"uid=1000 \"\n", None);
	assert!(!vol.contains("\nOptions="), "in:\n{vol}");
	assert!(
		vol.contains("PodmanArgs=--opt o=\"uid=1000 \""),
		"in:\n{vol}"
	);
}

#[test]
fn the_generator_turns_options_alone_into_opt_o() {
	let Some(generator) = generator() else {
		return;
	};
	let exec = generated_exec(
		&generator,
		&volume_unit("      o: uid=1000,gid=1000\n", None),
	)
	.expect("the generator dropped a unit with Options= and no Device=");
	assert!(exec.contains("--opt o=uid=1000,gid=1000 "), "{exec}");
}

#[test]
fn the_generator_keeps_a_unit_whose_type_has_no_device() {
	let Some(generator) = generator() else {
		return;
	};
	let exec = generated_exec(
		&generator,
		&volume_unit("      type: tmpfs\n      o: size=1g\n", None),
	)
	.expect("the generator dropped the unit");
	assert!(exec.contains("--opt type=tmpfs"), "{exec}");
	assert!(exec.contains("--opt o=size=1g"), "{exec}");
}
