//! A bare `KEY` in `environment:` is removed from the container, as it is under
//! `up`: the image's own value of the name must not survive. Quadlet has no key
//! for it, so the unit carries `PodmanArgs=--unsetenv=KEY` next to the bare
//! `Environment=KEY`.
//!
//! The first test checks the unit text and needs nothing installed. The others
//! run Podman's real Quadlet generator, then the argv it builds under the real
//! `podman run`, and skip with a `SKIP:` line where either is missing.

#![cfg(unix)]

use crate::backends::quadlet::tests::generator::generator;
use crate::backends::quadlet::tests::real_env::command;
use crate::parse_str;
use crate::quadlet::generate_at;

fn unit_of(yaml: &str) -> String {
	file_of(yaml, "proj-web.container")
}

fn file_of(yaml: &str, name: &str) -> String {
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));
	out.units
		.iter()
		.find(|u| u.filename == name)
		.unwrap()
		.contents
		.clone()
}

#[test]
fn a_bare_key_and_a_null_are_removed_and_a_value_is_kept() {
	let yaml = "services:\n  web:\n    image: img\n    environment:\n      BARE:\n      KEEP: value\n      NULLED: null\n";
	let unit = unit_of(yaml);
	assert!(unit.contains("Environment=BARE\n"), "{unit}");
	assert!(unit.contains("PodmanArgs=--unsetenv=\"BARE\"\n"), "{unit}");
	assert!(
		unit.contains("PodmanArgs=--unsetenv=\"NULLED\"\n"),
		"{unit}"
	);
	assert!(!unit.contains("KEEP=value"), "{unit}");
	let env = file_of(yaml, "proj-web.podup.env");
	assert!(env.ends_with("\nKEEP=value\n"), "{env}");
	assert!(!unit.contains("--unsetenv=\"KEEP\""), "{unit}");
}

#[test]
fn a_name_cannot_carry_another_flag() {
	let unit =
		unit_of("services:\n  web:\n    image: img\n    environment:\n      \"A --privileged\":\n");
	assert!(
		unit.contains("PodmanArgs=--unsetenv=\"A --privileged\"\n"),
		"{unit}"
	);
}

/// The `podman run` arguments Quadlet builds for the `proj-web` unit, without
/// the ones that only belong to a service, and the directory of the units,
/// which holds the env file the arguments name.
fn run_args(generator: &std::path::Path, yaml: &str) -> (tempfile::TempDir, Vec<String>) {
	let unit_dir = tempfile::tempdir().unwrap();
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));
	crate::quadlet::write_units(unit_dir.path(), &out.units).unwrap();
	let run = command(generator)
		.args(["-dryrun", "-user"])
		.env("QUADLET_UNIT_DIRS", unit_dir.path())
		.output()
		.unwrap();
	let text = String::from_utf8_lossy(&run.stdout).into_owned();
	let line = text
		.lines()
		.find_map(|l| l.strip_prefix("ExecStart="))
		.expect("no ExecStart line")
		.to_string();
	let mut words = line.split_whitespace().skip(2).map(str::to_string);
	let mut kept = Vec::new();
	while let Some(word) = words.next() {
		match word.as_str() {
			"--name" => {
				words.next();
			}
			"--replace" | "--cgroups=split" | "--sdnotify=conmon" | "-d" => {}
			_ => kept.push(word),
		}
	}
	(unit_dir, kept)
}

/// What `printenv container` prints, and whether it found the variable, in a
/// container run with the arguments of the unit. Podman sets `container=podman`
/// itself in every container, so the name is one every image has.
fn container_var(yaml: &str, host_value: Option<&str>) -> Option<Option<String>> {
	let generator = generator()?;
	if command("podman").arg("--version").output().is_err() {
		eprintln!("SKIP: podman is not installed");
		return None;
	}
	let (_units, args) = run_args(&generator, yaml);
	let mut run = command("podman");
	run.arg("run").arg("--rm").args(&args);
	match host_value {
		Some(v) => run.env("container", v),
		None => run.env_remove("container"),
	};
	let out = run.output().unwrap();
	if !out.status.success() && out.stdout.is_empty() && out.status.code() != Some(1) {
		panic!(
			"podman run {args:?} failed: {}",
			String::from_utf8_lossy(&out.stderr)
		);
	}
	Some(
		out.status
			.success()
			.then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()),
	)
}

const IMAGE: &str = "docker.io/library/busybox:latest";

fn yaml_with(environment: &str) -> String {
	format!(
		"services:\n  web:\n    image: {IMAGE}\n    command: [printenv, container]\n    environment:\n{environment}"
	)
}

#[test]
fn the_variable_is_gone_from_the_container() {
	let Some(found) = container_var(&yaml_with("      container:\n"), None) else {
		return;
	};
	assert_eq!(found, None, "the variable survived");
}

/// Without the removal Podman's own value stays: this is what the unit did
/// before, and what makes the test above mean something.
#[test]
fn a_service_that_does_not_name_the_variable_still_has_it() {
	let Some(found) = container_var(&yaml_with("      OTHER: x\n"), None) else {
		return;
	};
	assert_eq!(found.as_deref(), Some("podman"));
}

/// `Environment=KEY` takes the value of the process that starts the container
/// when there is one; the removal only applies to the image's value, so a
/// value the starter defines is kept.
#[test]
fn a_value_the_starting_process_defines_is_kept() {
	let Some(found) = container_var(&yaml_with("      container:\n"), Some("host")) else {
		return;
	};
	assert_eq!(found.as_deref(), Some("host"));
}

#[test]
fn a_value_given_in_the_file_is_the_value() {
	let Some(found) = container_var(&yaml_with("      container: given\n"), None) else {
		return;
	};
	assert_eq!(found.as_deref(), Some("given"));
}
