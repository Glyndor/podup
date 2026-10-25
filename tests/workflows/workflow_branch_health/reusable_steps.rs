//! The reusable checks out the tree before running the script it ships.
//!
//! Split out from `tests/workflows/workflow_branch_health.rs`; the reader is
//! inherited via `use super::*`.

use super::*;

#[test]
fn the_reusable_checks_out_the_tree_before_running_the_script() {
	// The step runs a script from this repository. Without a checkout the job
	// dies with `No such file or directory` and exit 127, which reads like a
	// broken script rather than a missing step; it happened on the first run of
	// this workflow, and the same omission cost two of the distribution
	// channels a red job earlier the same day.
	let reusable = read("reusable-branch-health.yml");
	let checkout = reusable
		.lines()
		.position(|l| l.contains("actions/checkout@"))
		.expect("the health job checks out the repository");
	let script = reusable
		.lines()
		.position(|l| l.contains("check-branch-conclusion.sh"))
		.expect("the health job runs the conclusion script");
	assert!(
		checkout < script,
		"the checkout has to come before the script that needs the tree"
	);
	assert!(
		reusable.contains("persist-credentials: false"),
		"nothing here writes, so the checkout must not leave a credential behind"
	);
}
