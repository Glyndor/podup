//! The lifecycle commands report what they created, do not report what they did
//! not remove, and say so when they had nothing to act on.
//!
//! Split out from `tests/cli/output/reporting_contract.rs`; the shared fixtures are
//! declared in the crate root, which the scenario files inherit via
//! `use super::*`.

use std::fs;
use std::process::Command;
use tempfile::tempdir;

use super::*;
use harness::{bin, podman_up, Project};

/// `up` reports the networks and volumes it creates, the way `down` has always
/// reported removing them.
///
/// It did not: measured on a four-object project, `up` created `p4_default`,
/// `p4_extra` and `p4_data` and named none of them, while `down -v` on the same
/// project listed all three as `Removed`. Creation went through
/// `tracing::info!` under the CLI's `warn` floor, so the two halves of the same
/// lifecycle disagreed about whether resources were worth mentioning.
///
/// Asserted on a fresh project name, because the create is idempotent: a second
/// `up` correctly reports nothing, so a reused project would pass this whether
/// or not the control exists.
#[tokio::test]
async fn up_reports_the_networks_and_volumes_it_creates() {
	if !podman_up().await {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("compose.yaml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    \
		 volumes:\n      - data:/data\nvolumes:\n  data:\nnetworks:\n  extra:\n",
	)
	.unwrap();
	let compose = compose.to_string_lossy().into_owned();
	let name = format!("t{}-mk", std::process::id());
	let out = Command::new(bin())
		.args(["-f", &compose, "-p", &name, "up", "-d"])
		.output()
		.expect("run podup up");
	let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
	let _ = Command::new(bin())
		.args(["-f", &compose, "-p", &name, "down", "-v"])
		.output();
	for needle in [
		&format!("Network {name}_default"),
		&format!("Network {name}_extra"),
		&format!("Volume {name}_data"),
	] {
		assert!(
			stderr.contains(needle.as_str()) && stderr.contains("Created"),
			"up must report creating {needle}; got stderr:\n{stderr}"
		);
	}
}

/// `down` only reports removals that happened.
///
/// It reported three that had not: measured on a project name that had never
/// been created, `down -v` printed `Network …_extra Removed`, `Network
/// …_default Removed` and `Volume …_data Removed`. The cause was `delete_ok`
/// discarding the boolean `delete_existed` returns, the very distinction that
/// method exists to preserve, and which the container path had always used, so
/// a 404 reached the caller as `Ok(())` and was announced as a deletion.
///
/// A volume is where this is worst: it names data the operator is being told is
/// gone. The reference prints nothing at all here and exits 0.
#[tokio::test]
async fn down_does_not_report_removing_what_never_existed() {
	if !podman_up().await {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("compose.yaml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    volumes:\n      - data:/data\nvolumes:\n  \
		 data:\nnetworks:\n  extra:\n",
	)
	.unwrap();
	let p = Project {
		compose: compose.to_string_lossy().into_owned(),
		name: format!("t{}-ghost", std::process::id()),
		_dir: dir,
	};
	let stderr = p.progress(&["down", "-v"]);
	assert!(
		!stderr.contains("Removed"),
		"nothing existed, so nothing may be reported as removed; got stderr:\n{stderr}"
	);
}

/// Every lifecycle command says so when it did nothing.
///
/// Measured before the fix on a project that was never created: `rm`, `stop`,
/// `restart`, `kill`, `pause` and `unpause` each printed zero lines and exited
/// 0, which reads exactly like success. Only `start` said anything. The cause is
/// that the per-replica query helpers (the bulk
/// [`Engine::live_project_replicas_sorted`] lookup used by `exec`/`cp`/
/// `port`/`logs`) fall back to the *static* compose names when nothing is
/// running, so each command dutifully walked a list of container names and
/// 404'd on every one of them.
#[tokio::test]
async fn idle_lifecycle_commands_say_they_did_nothing() {
	if !podman_up().await {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("compose.yaml");
	fs::write(&compose, "services:\n  web:\n    image: alpine:latest\n").unwrap();
	let p = Project {
		compose: compose.to_string_lossy().into_owned(),
		name: format!("t{}-idle", std::process::id()),
		_dir: dir,
	};
	for (args, verb) in [
		(vec!["rm", "-f"], "remove"),
		(vec!["stop"], "stop"),
		(vec!["start"], "start"),
		(vec!["restart"], "restart"),
		(vec!["kill"], "signal"),
		(vec!["pause"], "pause"),
		(vec!["unpause"], "unpause"),
	] {
		let stderr = p.progress(&args);
		assert!(
			stderr.contains(&format!("no containers to {verb}")),
			"`{}` on an uncreated project must say it did nothing; got stderr:\n{stderr}",
			args.join(" ")
		);
	}
}

/// The acceptance twin of the test above: when the command really does act, the
/// note must not appear. Without this pairing, a `note_if_idle` that fired
/// unconditionally would satisfy the rejection test and prove nothing.
#[tokio::test]
async fn lifecycle_commands_stay_quiet_when_they_did_act() {
	if !podman_up().await {
		return;
	}
	let p = Project::start("acted");
	for (args, verb) in [
		(vec!["restart"], "restart"),
		(vec!["stop"], "stop"),
		(vec!["rm", "-f"], "remove"),
	] {
		let stderr = p.progress(&args);
		assert!(
			!stderr.contains(&format!("no containers to {verb}")),
			"`{}` acted, so it must not claim there was nothing to do; got stderr:\n{stderr}",
			args.join(" ")
		);
	}
}
