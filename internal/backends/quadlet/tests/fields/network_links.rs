//! `links: [target:alias]` in the export. Podman has no link option, so `up`
//! carries it as a network alias of the target on the networks both services join
//! (see the Docker migration guide); the export writes the same alias on the
//! `Network=` line of the target's unit, so the units do what `up` does.

use std::process::Command;

use crate::backends::quadlet::tests::generator::generator;
use crate::parse_str;
use crate::quadlet::tests::unit_named;
use crate::quadlet::{generate_at, QuadletOutput};

fn export(yaml: &str) -> QuadletOutput {
	generate_at(
		&parse_str(yaml).unwrap(),
		"p",
		std::path::Path::new("/srv/app"),
	)
}

fn network_lines(out: &QuadletOutput, unit: &str) -> Vec<String> {
	unit_named(out, unit)
		.contents
		.lines()
		.filter(|l| l.starts_with("Network="))
		.map(str::to_string)
		.collect()
}

const BOTH_ON_BACK: &str = "services:\n  web:\n    image: x\n    networks: [back]\n    links: [\"db:database\"]\n  db:\n    image: x\n    networks: [back]\nnetworks:\n  back: {}\n";

#[test]
fn the_alias_goes_on_the_network_line_of_the_target_and_not_of_the_linker() {
	let out = export(BOTH_ON_BACK);
	assert_eq!(
		network_lines(&out, "p-db.container"),
		vec!["Network=p-back.network:alias=db,alias=database"]
	);
	assert_eq!(
		network_lines(&out, "p-web.container"),
		vec!["Network=p-back.network:alias=web"]
	);
}

#[test]
fn a_carried_link_is_not_reported_as_skipped_and_says_who_resolves_it() {
	let out = export(BOTH_ON_BACK);
	let joined = out.warnings.join("\n");
	assert!(!joined.contains("links skipped") && !joined.contains("links has no Quadlet"));
	assert!(
		joined.contains("web: link 'db:database' gives 'db' the network alias 'database'")
			&& joined.contains("every container on them resolves it, not only 'web'"),
		"{joined}"
	);
}

#[test]
fn a_link_of_a_job_gives_the_alias_too() {
	let yaml = "services:\n  db:\n    image: x\n    networks: [back]\njobs:\n  nightly:\n    image: x\n    networks: [back]\n    links: [\"db:database\"]\n    triggers:\n      schedule: ['0 3 * * *']\nnetworks:\n  back: {}\n";
	let out = export(yaml);
	assert_eq!(
		network_lines(&out, "p-db.container"),
		vec!["Network=p-back.network:alias=db,alias=database"]
	);
}

#[test]
fn the_alias_is_written_only_on_the_networks_both_services_join() {
	let yaml = "services:\n  web:\n    image: x\n    networks: [front]\n    links: [\"db:database\"]\n  db:\n    image: x\n    networks: [front, back]\nnetworks:\n  front: {}\n  back: {}\n";
	let out = export(yaml);
	assert_eq!(
		network_lines(&out, "p-db.container"),
		vec![
			"Network=p-front.network:alias=db,alias=database",
			"Network=p-back.network:alias=db"
		]
	);
}

#[test]
fn a_link_with_no_shared_network_or_no_such_service_adds_nothing_and_is_reported() {
	let yaml = "services:\n  web:\n    image: x\n    networks: [front]\n    links: [\"db:database\", \"ghost:g\"]\n  db:\n    image: x\n    networks: [back]\nnetworks:\n  front: {}\n  back: {}\n";
	let out = export(yaml);
	assert_eq!(
		network_lines(&out, "p-db.container"),
		vec!["Network=p-back.network:alias=db"]
	);
	let joined = out.warnings.join("\n");
	assert!(joined.contains("link 'db:database' is skipped"), "{joined}");
	assert!(joined.contains("link 'ghost:g' is skipped"), "{joined}");
}

#[test]
fn a_link_without_an_alias_adds_nothing_and_says_nothing() {
	let yaml = BOTH_ON_BACK.replace("db:database", "db");
	let out = export(&yaml);
	assert_eq!(
		network_lines(&out, "p-db.container"),
		vec!["Network=p-back.network:alias=db"]
	);
	assert!(
		!out.warnings.join("\n").contains("link"),
		"{:?}",
		out.warnings
	);
}

#[test]
fn an_alias_that_could_split_the_option_list_is_left_out() {
	let yaml = BOTH_ON_BACK.replace("db:database", "db:x,ip=10.0.0.9");
	let out = export(&yaml);
	assert_eq!(
		network_lines(&out, "p-db.container"),
		vec!["Network=p-back.network:alias=db"]
	);
	assert!(
		!unit_named(&out, "p-db.container")
			.contents
			.contains("10.0.0.9"),
		"an alias reached the list"
	);
}

#[test]
fn inside_a_pod_a_link_is_still_reported_as_skipped() {
	let yaml = format!("x-podman-pod: true\n{BOTH_ON_BACK}");
	let out = export(&yaml);
	assert!(
		out.warnings.iter().any(|w| w.starts_with("web: links")),
		"{:?}",
		out.warnings
	);
}

/// Podman's generator reads the line back into the `--network` flag with both
/// aliases, and a container started with those options answers to the alias
/// from another container of the network.
#[test]
fn podman_reads_the_alias_and_a_container_resolves_it() {
	let Some(generator) = generator() else {
		return;
	};
	let out = export(BOTH_ON_BACK);
	let dir = tempfile::tempdir().unwrap();
	for unit in &out.units {
		std::fs::write(dir.path().join(&unit.filename), &unit.contents).unwrap();
	}
	let run = Command::new(&generator)
		.args(["-dryrun", "-user"])
		.env("QUADLET_UNIT_DIRS", dir.path())
		.output()
		.unwrap();
	let stdout = String::from_utf8_lossy(&run.stdout);
	assert!(
		stdout.contains("--network p_back:alias=db,alias=database"),
		"{stdout}\n{}",
		String::from_utf8_lossy(&run.stderr)
	);
	real_resolution();
}

fn podman(args: &[&str]) -> std::process::Output {
	Command::new("podman").args(args).output().unwrap()
}

/// Two containers on a network, started with the option list the unit carries:
/// the second one resolves the alias of the first.
fn real_resolution() {
	if Command::new("podman").arg("--version").output().is_err() {
		return;
	}
	let id = std::process::id();
	let net = format!("t{id}links");
	let target = format!("t{id}links-db");
	let probe = format!("t{id}links-web");
	let cleanup = || {
		for name in [&target, &probe] {
			let _ = podman(&["rm", "-f", "-t", "0", name]);
		}
		let _ = podman(&["network", "rm", "-f", &net]);
	};
	cleanup();
	assert!(podman(&["network", "create", &net]).status.success());
	let options = format!("{net}:alias=db,alias=database");
	let started = podman(&[
		"run",
		"-d",
		"--name",
		&target,
		"--network",
		&options,
		"docker.io/library/busybox:latest",
		"sleep",
		"60",
	]);
	let resolved = started.status.success() && {
		let lookup = podman(&[
			"run",
			"--rm",
			"--name",
			&probe,
			"--network",
			&net,
			"docker.io/library/busybox:latest",
			"ping",
			"-c1",
			"-W1",
			"database",
		]);
		// A name that does not resolve is `bad address`; one that does is shown
		// with its address even where the ping itself is not allowed.
		String::from_utf8_lossy(&lookup.stdout).contains("PING database (")
	};
	cleanup();
	assert!(resolved, "the alias did not resolve");
}
