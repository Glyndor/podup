//! Mutually exclusive flag pairs are rejected at parse time rather than
//! silently letting one of them win.
//!
//! Split out from `tests/cli_diagnostics.rs`; the helper `bin` is inherited
//! via `use super::*`.

use super::*;

/// `--no-recreate` and `--force-recreate` are mutually exclusive: clap must
/// reject them together at parse time (exit non-zero) rather than silently
/// letting force-recreate win, matching docker compose.
#[test]
fn up_rejects_no_recreate_with_force_recreate() {
	let output = Command::new(bin())
		.args(["up", "--no-recreate", "--force-recreate"])
		.output()
		.expect("run up with conflicting flags");
	assert!(
		!output.status.success(),
		"conflicting recreate flags must be rejected"
	);
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(
		stderr.contains("cannot be used with") || stderr.contains("conflict"),
		"expected a clap conflict error; got:\n{stderr}"
	);
}

/// The same exclusivity holds for `create`.
#[test]
fn create_rejects_no_recreate_with_force_recreate() {
	let output = Command::new(bin())
		.args(["create", "--no-recreate", "--force-recreate"])
		.output()
		.expect("run create with conflicting flags");
	assert!(
		!output.status.success(),
		"conflicting recreate flags must be rejected"
	);
	// Which rejection, not just that one happened. A non-zero exit here is
	// satisfied by any failure (a missing compose file, an unreachable Podman,
	// a panic) none of which exercise the flag conflict this test is named for.
	let err = String::from_utf8_lossy(&output.stderr);
	assert!(
		err.contains("--no-recreate") && err.contains("--force-recreate"),
		"the rejection did not name the two conflicting flags: {err:?}"
	);
}
