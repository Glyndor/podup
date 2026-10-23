//! `pull` reports both ends through the progress layer, once per image, and a
//! piped `pull` keeps the plain lines it always printed.
//!
//! Split out from `tests/reporting_contract.rs`; the shared fixtures are
//! declared in the crate root, which the scenario files inherit via
//! `use super::*`.

use std::fs;
use tempfile::tempdir;

use super::*;
use harness::{podman_up, Project};

/// `pull` reports through the progress layer, with a matching `Pulled`.
///
/// It used a bare `eprintln!`, the one user-facing line in the binary that
/// bypassed `ui` entirely, so it ignored `PROGRESS_ENABLED` and an embedder
/// asking podup to stay silent got it anyway. There was never a `Pulled` at all,
/// so a pull that finished looked exactly like one that hung.
#[tokio::test]
async fn pull_reports_both_ends() {
	if !podman_up().await {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("compose.yaml");
	fs::write(&compose, "services:\n  x:\n    image: alpine:latest\n").unwrap();
	let p = Project {
		compose: compose.to_string_lossy().into_owned(),
		name: format!("t{}-pull", std::process::id()),
		_dir: dir,
	};
	let stderr = p.progress(&["pull"]);
	assert!(
		stderr.contains("Pulling") && stderr.contains("Pulled"),
		"pull must report starting and finishing; got:\n{stderr}"
	);
	assert_eq!(
		stderr.matches("Pulling").count(),
		1,
		"one image, one Pulling; got:\n{stderr}"
	);
}

/// One image, one `Pulling`, even on the `up` path.
///
/// `up` warms the image cache before the per-level walk and then pulls again
/// inside it, and both reported, so `Pulling` appeared twice per image on `up`
/// while a standalone `pull` printed it once. The prefetch is best-effort
/// warming and the walk's own pull is the authoritative one, so only the second
/// speaks.
///
/// `pull_policy: always` is what forces both stages to actually issue a pull for
/// an image that is already local; with the default `missing` the prefetch would
/// short-circuit and the duplication could not appear whether or not the control
/// exists.
#[tokio::test]
async fn up_pulls_an_image_once() {
	if !podman_up().await {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("compose.yaml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    pull_policy: always\n    command: \
		 [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	let p = Project {
		compose: compose.to_string_lossy().into_owned(),
		name: format!("t{}-once", std::process::id()),
		_dir: dir,
	};
	let stderr = p.progress(&["up", "-d"]);
	let pulls = stderr.matches("Pulling").count();
	assert!(
		pulls <= 1,
		"one image must produce at most one Pulling on up; got {pulls} in:\n{stderr}"
	);
}

/// A `pull` whose stderr is a pipe keeps the plain lines it always printed.
///
/// `pull` now opens the same board `up` does (#1671), and the board owns the
/// rendering once it is open. Off a terminal that must change nothing: the
/// events go out as ` Image <ref>  Pulling` / ` Pulled`, one line each, with no
/// cursor hiding, no cursor moves and no spinner. Animation in a CI log is a
/// defect, and so is a log that says less than it used to.
///
/// `Command::output` gives the child a pipe for stderr, which is exactly the
/// condition being asserted.
#[tokio::test]
async fn a_piped_pull_keeps_the_plain_lines() {
	if !podman_up().await {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("compose.yaml");
	fs::write(&compose, "services:\n  x:\n    image: alpine:latest\n").unwrap();
	let p = Project {
		compose: compose.to_string_lossy().into_owned(),
		name: format!("t{}-pipe", std::process::id()),
		_dir: dir,
	};
	let stderr = p.progress(&["pull"]);
	assert!(
		stderr.contains(" Image alpine:latest  Pulling"),
		"the plain line is unchanged in a pipe; got:\n{stderr}"
	);
	assert!(
		stderr.contains(" Image alpine:latest  Pulled"),
		"and so is its closing line; got:\n{stderr}"
	);
	assert!(
		!stderr.contains('\x1b'),
		"a pipe gets no escape sequence at all, board or otherwise; got:\n{stderr:?}"
	);
	for marker in ["⠿", "✔", "[+] Running"] {
		assert!(
			!stderr.contains(marker),
			"a pipe gets no board furniture ({marker}); got:\n{stderr}"
		);
	}
}
