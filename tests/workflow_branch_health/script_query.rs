//! Property 2: the API query the conclusion script makes, and the reader that
//! reads it.
//!
//! Split out from `tests/workflow_branch_health.rs`; the script reader is
//! local, only this file needs it.

use std::fs;
use std::path::Path;

/// Read a file from `.github/scripts/`. The API shape assertions moved
/// here from the reusable when the `gh api` call moved into the
/// script the reusable calls.
fn read_script(name: &str) -> String {
	let path = Path::new(env!("CARGO_MANIFEST_DIR"))
		.join(".github/scripts")
		.join(name);
	fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name} is readable: {e}"))
}

#[test]
fn the_script_picks_the_run_for_the_branch_head_and_keeps_a_buffer_to_sort() {
	// The API call shape moved into the script the reusable runs:
	// the verdict is the run for the branch's CURRENT head, and the
	// script does the polling itself. The runs URL must keep
	// `per_page=30` (a one-item page hands back whatever lands at
	// index 0, which the API does not guarantee is the newest run);
	// the script must filter by `head_sha` so an older green run for
	// a different commit cannot answer for the branch (the defect
	// measured 2026-09-20); it must filter by `event=push` so a
	// `pull_request` run whose source branch is the watched branch
	// cannot shadow the push run for the same head_sha (the defect
	// measured 2026-09-20 on the 5.9.5 release push for commit
	// 83315af); and it must NOT ask for `status=success` alone, which
	// would miss every failing run.
	let script = read_script("check-branch-conclusion.sh");
	let has_query = script.lines().any(|l| {
		let trimmed = l.trim_start();
		if trimmed.starts_with('#') {
			return false;
		}
		trimmed.contains("per_page=30")
	});
	assert!(
		has_query,
		"check-branch-conclusion.sh's `gh api` call no longer asks for \
		 per_page=30. A one-item page hands back whatever lands at \
		 index 0; a newer verdict for the head would be missed."
	);

	assert!(
		script.contains("head_sha"),
		"check-branch-conclusion.sh does not filter runs by head_sha. \
		 The verdict is the run for the branch's current head, not \
		 the newest run of any status. Without head_sha, an older \
		 green run for a different commit answers for the branch."
	);

	let has_event_push = script.lines().any(|l| {
		let trimmed = l.trim_start();
		if trimmed.starts_with('#') {
			return false;
		}
		trimmed.contains("event=push")
	});
	assert!(
		has_event_push,
		"check-branch-conclusion.sh's `gh api` call no longer asks for \
		 event=push. A pull_request run on a PR whose source branch is \
		 the watched branch carries the same head_sha as the push run \
		 for that head; without `event=push` the API returns both, \
		 the newer pull_request run wins, and the gate reads a run \
		 the branch did not trigger."
	);

	let has_event_in_jq = script.lines().any(|l| {
		let trimmed = l.trim_start();
		if trimmed.starts_with('#') {
			return false;
		}
		trimmed.contains(".event == \"push\"") || trimmed.contains(".event==\"push\"")
	});
	assert!(
		has_event_in_jq,
		"check-branch-conclusion.sh's jq filter does not require \
		 .event == \"push\". The URL's `event=push` keeps the page \
		 from filling with runs the filter would discard, but the jq \
		 filter is what tests a stubbed API and what an operator \
		 reading the file sees as the canonical truth. Both halves \
		 of the filter are required: the URL alone is an optimization."
	);

	let has_success_only = script.lines().any(|l| {
		let trimmed = l.trim_start();
		if trimmed.starts_with('#') {
			return false;
		}
		trimmed.contains("status=success")
	});
	assert!(
		!has_success_only,
		"check-branch-conclusion.sh asks for status=success on its \
		 own. That is the freshness watcher's shape and cannot catch \
		 this defect: a failing run is missing from its answer by \
		 construction."
	);
}
