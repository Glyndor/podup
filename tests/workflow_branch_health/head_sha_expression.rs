//! The `HEAD_SHA:` line of the reusable gates the commit to the branch under
//! test, with a planted negative control for the shape that decides it.
//!
//! Split out from `tests/workflow_branch_health.rs`; the reader is
//! inherited via `use super::*`.

use super::*;

/// The expression on the `HEAD_SHA:` line of the branch-health reusable.
/// Scoped to that one line so a `contains "inputs.branch"` check that
/// also matches the comment block above, or a copy in the script the
/// reusable runs, cannot satisfy it.
fn head_sha_expression(reusable: &str) -> String {
	let line = reusable
		.lines()
		.find(|l| l.trim_start().starts_with("HEAD_SHA:"))
		.unwrap_or_else(|| panic!("the reusable has a HEAD_SHA: env line"));
	let after = line.trim_start().trim_start_matches("HEAD_SHA:").trim();
	let open = after
		.find("${{")
		.unwrap_or_else(|| panic!("HEAD_SHA: uses a ${{ }} expression"));
	let close_rel = after[open + 3..]
		.find("}}")
		.unwrap_or_else(|| panic!("HEAD_SHA: ${{ }} expression is closed"));
	let close = open + 3 + close_rel;
	after[open + 3..close].trim().to_string()
}

/// The expression that gates `HEAD_SHA` to the branch under test on
/// push. Equality (not `contains`) is what catches a wrong shape that
/// still happens to name the right substrings.
fn expected_head_sha_expression() -> String {
	"(github.event_name == 'push' && github.ref_name == inputs.branch) && github.sha || ''"
		.to_string()
}

// The reusable must compare `github.ref_name` against `inputs.branch`
// on its `HEAD_SHA:` line, so a push to a different branch does not
// hand the script a commit that no run on the watched branch carries.
// The defect measured on 2026-09-20 made `health-main` run for eight
// minutes on every push to `develop` before failing; the next pull
// request then read that failure and failed in eight seconds.
#[test]
fn head_sha_expression_compares_pushed_branch_with_branch_under_test() {
	let reusable = read("reusable-branch-health.yml");
	let expr = head_sha_expression(&reusable);
	assert_eq!(
		expr,
		expected_head_sha_expression(),
		"the `HEAD_SHA:` line no longer compares `github.ref_name` \
		 against `inputs.branch`. The two callers in `ci.yml` run \
		 cross-branch on purpose: a push to `develop` fires \
		 `health-main`, which would then look for a run on `main` \
		 whose `head_sha` is `develop`'s commit. No such run exists, \
		 so the poll loop exhausts itself and the run fails. The \
		 branch comparison gates `HEAD_SHA` to the branch under \
		 test; without it the script receives a commit no run on \
		 the watched branch carries."
	);
}

// Pin the assertion on input that resembles the reusable's step block
// but with wrong expressions. A check that always passed would
// satisfy the structural assertion above and prove nothing.
#[test]
fn the_head_sha_shape_catches_a_missing_branch_comparison() {
	// The original buggy shape: `push` and `github.sha` only. The
	// `BRANCH: ${{ inputs.branch }}` env below is what a naive
	// `contains "inputs.branch"` check would latch onto.
	let reusable_orig = "\
      - name: Check the run for the branch's current head
        env:
          GH_TOKEN: ${{ github.token }}
          WORKFLOW: ${{ inputs.workflow }}
          BRANCH: ${{ inputs.branch }}
          HEAD_SHA: ${{ github.event_name == 'push' && github.sha || '' }}
        run: bash .github/scripts/check-branch-conclusion.sh
";
	let expr_orig = head_sha_expression(reusable_orig);
	assert_ne!(
		expr_orig,
		expected_head_sha_expression(),
		"a buggy expression without a branch comparison was accepted \
		 as the expected shape"
	);

	// The expected shape, parsed from a hand-built step that does
	// not name `inputs.branch` anywhere outside the HEAD_SHA line.
	let reusable_ok = "\
      - name: Check the run for the branch's current head
        env:
          WORKFLOW: ${{ inputs.workflow }}
          HEAD_SHA: ${{ (github.event_name == 'push' && github.ref_name == inputs.branch) && github.sha || '' }}
        run: bash .github/scripts/check-branch-conclusion.sh
";
	assert_eq!(
		head_sha_expression(reusable_ok),
		expected_head_sha_expression(),
		"the planted correct expression did not parse to the expected \
		 shape; the parser is broken"
	);
}
