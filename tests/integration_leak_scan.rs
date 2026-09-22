//! Integration-leak scanner, run as a separate process after
//! `engine_integration` has exited.
//!
//! ## Why a separate binary
//!
//! A scanner inside `engine_integration` cannot tell a leak from a test
//! that is still running. cargo runs the tests of one binary in parallel,
//! so no test in it, whatever its name, is guaranteed to run after the
//! others: a `z_`-named scanner there was measured on the `podman-vm (5)`
//! lane reporting, as leaks, two resources of a test that finished after
//! it. "After the suite" is not a moment that exists inside the suite when
//! the runner is parallel.
//!
//! Cargo runs each `--test` target as its own process. A second
//! `cargo test --test <other>` invocation cannot overlap the first, so the
//! scanner living in this binary genuinely runs after `engine_integration`
//! has exited. That removes the race without adding a barrier or a sleep.
//!
//! ## How the PID crosses the process boundary
//!
//! `engine_integration` knows its own PID; this binary does not. The PID
//! crosses through a fixed file the suite writes and the CI step reads. The
//! file path is one constant on each side; the env var `PODUP_LEAK_SCAN_PID`
//! carries the value.
//!
//! Only the PID crosses. The scanner rebuilds the three patterns
//! (`t{pid}-`, `podup-test-*-{pid}`, `podup-commit-test-{pid}`) from it,
//! because the patterns have no business outside this file.
//!
//! ## What this binary does NOT do
//!
//! When `PODUP_LEAK_SCAN_PID` is unset, this binary prints a single line and
//! returns. A skipped test has not run, and a green run of this target with
//! no PID would read as "scanned, found nothing". The log line distinguishes
//! the two; the CI step proves the scan ran.

#![allow(clippy::items_after_statements)]

use std::process::Command;

/// Fixed path `engine_integration` writes its PID to. The CI step reads the
/// same path when setting `PODUP_LEAK_SCAN_PID`; one constant, two readers,
/// no string to keep in step.
///
/// Documented here as a contract marker rather than read at runtime: this
/// binary gets the PID through `PODUP_LEAK_SCAN_PID`, not from the file, so
/// the constant has no caller inside this crate. The same path lives in
/// `tests/engine_integration.rs` as `PID_FILE_PATH` and is used by
/// `publish_pid_file`, and in `.github/workflows/podman-lane.yml` as the
/// heredoc arg to `cat` — one literal, three readers.
#[allow(dead_code)]
const PID_FILE_PATH: &str = "target/podup-leak-scan-pid";

/// Run `podman <args>` and return the stdout lines, or panic with the
/// command and its stderr on non-zero exit.
///
/// A `podman` invocation that exits non-zero fails the scan. A scanner
/// that returned nothing on a failed `podman ls` would read as "no leaks"
/// exactly when it cannot see, and the run would pass while the leak it
/// should have caught stayed on the host. An empty list would be
/// indistinguishable from "podman answered and there is nothing".
fn podman_lines(args: &[&str]) -> Vec<String> {
	let out = Command::new("podman")
		.args(args)
		.output()
		.unwrap_or_else(|e| panic!("podman {args:?} failed to spawn: {e}"));
	assert!(
		out.status.success(),
		"podman {args:?} exited {}: {}",
		out.status,
		String::from_utf8_lossy(&out.stderr)
	);
	String::from_utf8_lossy(&out.stdout)
		.lines()
		.map(str::to_owned)
		.collect()
}

/// Strip the registry prefix podman prepends to image repositories
/// (`docker.io/library/...`, `localhost/...`). The matching happens on the
/// bare name so the prefix arithmetic stays trivial.
fn short_image_name(repo: &str) -> &str {
	match repo.rsplit('/').next() {
		Some(name) => name,
		None => repo,
	}
}

/// Every resource on the host that carries the per-run prefix derived from
/// `pid`. The string is `<kind>: <name>` so a failing assertion can name
/// the offender rather than just say "something leaked".
///
/// The leak check never touches or reports resources that do not carry
/// this run's prefix. The three patterns it does match: `t{pid}-` for
/// project names, `podup-test-*-{pid}` as the tail every explicit image
/// tag ends with, and `podup-commit-test-{pid}` for the `commit` target.
///
/// A failed `podman` invocation is a panic, not an empty list. See
/// [`podman_lines`].
fn find_per_run_leaks(pid: u32) -> Vec<String> {
	let pid_prefix = format!("t{pid}-");
	let image_tail = format!("-{pid}");
	let commit_prefix = format!("podup-commit-test-{pid}");
	let mut leaks = Vec::new();

	for repo in podman_lines(&["image", "ls", "--format", "{{.Repository}}"]) {
		let name = short_image_name(&repo);
		if name.starts_with(&pid_prefix)
			|| (name.starts_with("podup-test-") && name.ends_with(&image_tail))
			|| name.starts_with(&commit_prefix)
		{
			leaks.push(format!("image: {repo}"));
		}
	}

	for name in podman_lines(&["container", "ls", "-a", "--format", "{{.Names}}"]) {
		if name.starts_with(&pid_prefix) {
			leaks.push(format!("container: {name}"));
		}
	}

	for name in podman_lines(&["volume", "ls", "--format", "{{.Name}}"]) {
		if name.starts_with(&pid_prefix) {
			leaks.push(format!("volume: {name}"));
		}
	}

	for name in podman_lines(&["network", "ls", "--format", "{{.Name}}"]) {
		if name.starts_with(&pid_prefix) {
			leaks.push(format!("network: {name}"));
		}
	}

	for name in podman_lines(&["pod", "ls", "--format", "{{.Name}}"]) {
		if name.starts_with(&pid_prefix) {
			leaks.push(format!("pod: {name}"));
		}
	}

	leaks
}

/// Whether the engine has a Podman to talk to. The shape
/// `engine_integration::podman()` uses, kept here rather than imported
/// because each `--test` target is its own crate.
async fn podman_reachable() -> bool {
	match podup::podman::connect_from_env().or_else(|_| podup::podman::connect(None)) {
		Ok(client) => client.ping().await.is_ok(),
		Err(_) => false,
	}
}

/// The actual scan.
///
/// Reads `PODUP_LEAK_SCAN_PID` (the CI step sets it from
/// [`PID_FILE_PATH`]) and fails listing every resource carrying the
/// corresponding prefix.
///
/// When the env var is unset, prints a single line and returns. A skipped
/// test has not run; a green run of this target with no PID would read as
/// "scanned, found nothing". The line distinguishes the two. The CI step,
/// not this test, is what proves the scan ran.
#[tokio::test]
async fn scan_for_per_run_leaks() {
	let pid_str = match std::env::var_os("PODUP_LEAK_SCAN_PID") {
		Some(s) if !s.is_empty() => s,
		_ => {
			eprintln!("PODUP_LEAK_SCAN_PID not set; the leak scan was not requested for this run.");
			return;
		}
	};
	let pid: u32 = pid_str
		.to_string_lossy()
		.parse()
		.unwrap_or_else(|e| panic!("PODUP_LEAK_SCAN_PID is not a u32: {e}"));
	let leaks = find_per_run_leaks(pid);
	assert!(
		leaks.is_empty(),
		"resources with the per-run prefix t{pid}- remain on the host:\n  {}",
		leaks.join("\n  ")
	);
}

/// Guard for the scanner itself.
///
/// Replacing `find_per_run_leaks` with an empty body would leave the
/// detector passing: the suite has no other test that asks the scanner
/// to find anything, so a no-op scanner would never be noticed. This
/// test closes that gap.
///
/// Plants a real network carrying this process's own `t{pid}-` prefix,
/// asks the scanner for leaks under that PID, asserts the planted network
/// appears in the result by name, then removes it in a drop guard so a
/// failing assertion still cleans up.
///
/// Skipped when Podman is unreachable, matching the other integration
/// tests. Asserting on an empty scanner with no daemon would be a green lie.
#[tokio::test]
async fn the_scanner_detects_a_planted_network() {
	if !podman_reachable().await {
		return;
	}

	let pid = std::process::id();
	let net_name = format!("t{pid}-leak-scan-guard");

	/// Removes the planted network on drop, including the path where the
	/// assertion below fails. A panic in the test body would otherwise
	/// leave the network on the host, and the next run would fail for a
	/// reason that has nothing to do with the test.
	struct NetworkGuard(String);
	impl Drop for NetworkGuard {
		fn drop(&mut self) {
			let _ = Command::new("podman")
				.args(["network", "rm", "-f", &self.0])
				.output();
		}
	}

	let created = Command::new("podman")
		.args(["network", "create", &net_name])
		.status()
		.expect("podman network create");
	assert!(
		created.success(),
		"could not plant the guard network; podman network create {net_name} failed"
	);
	let _guard = NetworkGuard(net_name.clone());

	let leaks = find_per_run_leaks(pid);
	assert!(
		leaks.iter().any(|l| l.contains(&net_name)),
		"planted network {net_name} not in scan result: {leaks:?}"
	);
}

/// A `podman` invocation that exits non-zero fails the scan instead of
/// reading as an empty list.
///
/// The scanner is only honest if it can tell "nothing is there" from "I
/// could not look". A helper that turned a failed `podman ... ls` into an
/// empty `Vec` would report "no leaks" exactly when it saw nothing, and no
/// other test here can notice, because on a healthy host every `ls` it runs
/// succeeds. This one drives the failure on purpose: Podman rejects an
/// undefined template function with exit 125
/// (`template: list:1: function "garbage" not defined`, Podman 5.7.0).
///
/// The panic message is read, not just the fact of a panic, so the test
/// pins the non-zero-exit branch and not the spawn-failure one. Skipped when
/// there is no `podman` binary to run at all.
#[test]
fn a_failed_podman_command_fails_the_scan() {
	let has_podman = Command::new("podman")
		.arg("--version")
		.output()
		.is_ok_and(|o| o.status.success());
	if !has_podman {
		return;
	}
	let outcome =
		std::panic::catch_unwind(|| podman_lines(&["image", "ls", "--format", "{{garbage}}"]));
	let payload = match outcome {
		Ok(lines) => panic!(
			"a podman command that exits non-zero must fail the scan, \
			 but it returned {} line(s): {lines:?}",
			lines.len()
		),
		Err(payload) => payload,
	};
	let message = payload
		.downcast_ref::<String>()
		.cloned()
		.or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
		.unwrap_or_default();
	assert!(
		message.contains("exited"),
		"the failure must name the non-zero exit, not a spawn error: {message}"
	);
}
