//! Further CLI binary integration tests (kept separate from cli2.rs to stay
//! under the source line limit).
use std::fs;
use std::process::Command;
use tempfile::tempdir;

use super::*;

#[tokio::test]
async fn cli_logs_tail_limits_output() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-logstail", std::process::id());
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"for i in 1 2 3 4 5; do echo line-$i; done; sleep infinity\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	Command::new(bin())
		.args(["-f", c, "-p", &proj, "up", "-d"])
		.output()
		.unwrap();
	// Give the container a moment to emit its lines.
	tokio::time::sleep(std::time::Duration::from_millis(800)).await;

	let logs = Command::new(bin())
		.args(["-f", c, "-p", &proj, "logs", "--tail", "2"])
		.output()
		.unwrap();
	assert!(logs.status.success(), "logs failed: {:?}", logs.stderr);
	let lines = String::from_utf8_lossy(&logs.stdout)
		.lines()
		.filter(|l| l.contains("line-"))
		.count();
	assert_eq!(lines, 2, "logs --tail 2 must show exactly 2 lines");

	Command::new(bin())
		.args(["-f", c, "-p", &proj, "down"])
		.output()
		.unwrap();
}
