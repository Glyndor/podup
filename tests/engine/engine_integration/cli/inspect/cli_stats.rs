//! CLI binary integration tests for `stats --no-stream`.
use super::*;
use std::fs;
use std::process::Command;
use tempfile::tempdir;
#[tokio::test]
async fn cli_stats_no_stream_reports_running_container() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-stats", std::process::id());
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	Command::new(bin())
		.args(["-f", c, "-p", &proj, "up", "-d"])
		.output()
		.unwrap();

	let stats = Command::new(bin())
		.args(["-f", c, "-p", &proj, "stats", "--no-stream"])
		.output()
		.unwrap();
	assert!(
		stats.status.success(),
		"stats failed: {}",
		String::from_utf8_lossy(&stats.stderr)
	);
	let out = String::from_utf8_lossy(&stats.stdout);
	assert!(out.contains("CPU %"), "stats must print a header: {out}");
	assert!(
		out.contains(&format!("{proj}-web")),
		"stats must list the running container: {out}"
	);

	Command::new(bin())
		.args(["-f", c, "-p", &proj, "down"])
		.output()
		.unwrap();
}
