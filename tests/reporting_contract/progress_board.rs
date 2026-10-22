//! The progress board: a pipe gets the events and never the animation, the
//! board writes to stderr only, and `--ansi never` gets no board at all.
//!
//! Split out from `tests/reporting_contract.rs`; the shared fixtures are
//! declared in the crate root, which the scenario files inherit via
//! `use super::*`.

use std::fs;
use tempfile::tempdir;

use super::*;
use harness::{podman_up, Project};

/// A pipe gets the events, never the animation.
///
/// This is the contract that protects CI logs. The live region only exists when
/// stderr is a terminal and colour is on; anywhere else the same event model
/// comes out as append-only lines. **Animation in a CI log is a defect**, and
/// so is a CI log that says less than the terminal did, which is why the
/// intermediate transitions are asserted here too. Before the board they did
/// not exist at all: `up` reported only the finished state of each resource.
#[tokio::test]
async fn a_piped_up_gets_transitions_and_no_escapes() {
	if !podman_up().await {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("compose.yaml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	let p = Project {
		compose: compose.to_string_lossy().into_owned(),
		name: format!("t{}-pipe", std::process::id()),
		_dir: dir,
	};
	let stderr = p.progress(&["up", "-d"]);
	assert!(
		!stderr.contains('\u{1b}'),
		"a pipe must get no escape sequences at all; got:\n{stderr:?}"
	);
	// Asserted per resource, not as "some line somewhere says Creating". A
	// looser version of this passed with the container's start event deleted,
	// because the network's own `Creating` satisfied it.
	let has = |name: &str, verbs: &[&str]| {
		stderr
			.lines()
			.any(|l| l.contains(name) && verbs.iter().any(|v| l.contains(v)))
	};
	let container = format!("{}-web-1", p.name);
	assert!(
		has(&container, &["Starting", "Creating"]),
		"the container needs its own transition, not only its ending; got:\n{stderr}"
	);
	assert!(
		has(&container, &["Started", "Created", "Running"]),
		"and it must still get its ending; got:\n{stderr}"
	);
	let network = format!("{}_default", p.name);
	assert!(
		has(&network, &["Creating"]),
		"the network needs one too; got:\n{stderr}"
	);
}

/// `--ansi never` means it. The board is gated on the colour choice as well as
/// on the terminal, because someone who asked for no escapes did not ask for a
/// quieter kind of escape.
#[tokio::test]
async fn ansi_never_gets_no_board() {
	if !podman_up().await {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("compose.yaml");
	fs::write(&compose, "services:\n  web:\n    image: alpine:latest\n").unwrap();
	let p = Project {
		compose: compose.to_string_lossy().into_owned(),
		name: format!("t{}-noansi", std::process::id()),
		_dir: dir,
	};
	let stderr = p.progress(&["--ansi", "never", "up", "-d"]);
	assert!(
		!stderr.contains('\u{1b}'),
		"--ansi never must emit nothing to repaint with; got:\n{stderr:?}"
	);
}

/// The board writes to stderr only. stdout stays a clean pipe, which is what
/// lets `run -d` keep printing its container id there and `config` keep piping
/// into a file.
#[tokio::test]
async fn the_board_never_touches_stdout() {
	if !podman_up().await {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("compose.yaml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	let p = Project {
		compose: compose.to_string_lossy().into_owned(),
		name: format!("t{}-stdout", std::process::id()),
		_dir: dir,
	};
	let stdout = p.run(&["up", "-d"]);
	assert!(
		stdout.trim().is_empty(),
		"up must leave stdout empty; got:\n{stdout}"
	);
}

/// `down` gets a board too, seeded from what Podman actually has rather than
/// from what the compose file describes: a service the file lists but that was
/// never created would sit on the board as a row that never moves.
#[tokio::test]
async fn a_piped_down_gets_transitions_for_what_exists() {
	if !podman_up().await {
		return;
	}
	let p = Project::start("dwn");
	let stderr = p.progress(&["down", "-v"]);
	assert!(
		!stderr.contains('\u{1b}'),
		"a pipe must get no escape sequences; got:\n{stderr:?}"
	);
	let has = |name: &str, verb: &str| stderr.lines().any(|l| l.contains(name) && l.contains(verb));
	let container = format!("{}-web-1", p.name);
	assert!(
		has(&container, "Stopping"),
		"the container needs its transition; got:\n{stderr}"
	);
	assert!(has(&container, "Removed"), "and its ending; got:\n{stderr}");
	let network = format!("{}_default", p.name);
	assert!(
		has(&network, "Removing") && has(&network, "Removed"),
		"the network needs both; got:\n{stderr}"
	);
}
