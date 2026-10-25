//! CLI binary integration tests for the `logs --tail` flag.
use super::*;
use std::fs;
use std::process::Command;
use tempfile::tempdir;
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

	// Poll until the container has emitted its lines instead of sleeping a fixed
	// duration: `logs --tail 2` must eventually show exactly 2 `line-` rows.
	let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
	let mut logs;
	loop {
		logs = Command::new(bin())
			.args(["-f", c, "-p", &proj, "logs", "--tail", "2"])
			.output()
			.unwrap();
		let lines = String::from_utf8_lossy(&logs.stdout)
			.lines()
			.filter(|l| l.contains("line-"))
			.count();
		if logs.status.success() && lines == 2 {
			break;
		}
		if tokio::time::Instant::now() >= deadline {
			break;
		}
		tokio::time::sleep(std::time::Duration::from_millis(100)).await;
	}
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
