//! CLI integration tests for `stats` table-shaping flags (`--all`, `--no-trunc`,
//! `--format`). Kept in its own submodule so `cli_commands.rs` stays within the
//! source line limit.
use std::fs;
use std::process::Command;
use tempfile::tempdir;

use super::*;

#[tokio::test]
async fn cli_stats_flags_truncate_all_and_format() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	// A service name long enough that `{proj}-{svc}-1` overflows the 32-char NAME
	// column, so truncation/`--no-trunc` is observable.
	let svc = "superlongservicenamethatoverflows";
	let proj = format!("t{}-stf", std::process::id());
	fs::write(
		&compose,
		format!(
			"services:\n  {svc}:\n    image: alpine:latest\n    pull_policy: never\n    command: [\"sleep\", \"infinity\"]\n"
		),
	)
	.unwrap();
	let c = compose.to_str().unwrap();
	// A single-replica service's container is named `{proj}-{svc}` (no `-1`).
	let full = format!("{proj}-{svc}");

	Command::new(bin())
		.args(["-f", c, "-p", &proj, "up", "-d"])
		.output()
		.unwrap();

	// `--no-trunc` shows the full container name; the default table truncates it
	// (ellipsis present, full name gone).
	let notrunc = run(c, &proj, &["stats", "--no-stream", "--no-trunc"]);
	assert!(
		notrunc.contains(&full),
		"--no-trunc must show full name: {notrunc}"
	);

	let truncd = run(c, &proj, &["stats", "--no-stream"]);
	assert!(
		truncd.contains('…'),
		"default table must truncate: {truncd}"
	);
	assert!(
		!truncd.contains(&full),
		"default must not show full name: {truncd}"
	);

	// `--format json` emits a JSON array with the full (untruncated) name.
	let json = run(c, &proj, &["stats", "--no-stream", "--format", "json"]);
	assert!(
		json.contains("\"Name\""),
		"json must have Name field: {json}"
	);
	assert!(
		json.contains("\"CPUPerc\""),
		"json must have CPUPerc: {json}"
	);
	assert!(json.contains(&full), "json must carry full name: {json}");

	// Stop the container: it drops out of the default (running-only) view but
	// `--all` folds it back in as a zeroed row.
	Command::new(bin())
		.args(["-f", c, "-p", &proj, "stop"])
		.output()
		.unwrap();
	let stopped = run(c, &proj, &["stats", "--no-stream", "--no-trunc"]);
	assert!(
		!stopped.contains(&full),
		"stopped container must be hidden: {stopped}"
	);
	let all = run(c, &proj, &["stats", "--no-stream", "--all", "--no-trunc"]);
	assert!(
		all.contains(&full),
		"--all must include stopped container: {all}"
	);

	Command::new(bin())
		.args(["-f", c, "-p", &proj, "down"])
		.output()
		.unwrap();
}

/// Run the built `podup` against compose file `c`/project `proj` with `args` and
/// return its stdout, asserting a clean exit.
fn run(c: &str, proj: &str, args: &[&str]) -> String {
	let mut full = vec!["-f", c, "-p", proj];
	full.extend_from_slice(args);
	let out = Command::new(bin()).args(&full).output().unwrap();
	assert!(
		out.status.success(),
		"podup {args:?} failed: {:?}",
		out.stderr
	);
	String::from_utf8_lossy(&out.stdout).into_owned()
}
