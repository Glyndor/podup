//! CLI binary integration tests for `top` and `images`.
use super::*;
use std::fs;
use std::process::Command;
use tempfile::tempdir;
#[tokio::test]
async fn cli_top_subcommand() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-cltop", std::process::id());
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&proj,
			"up",
			"--detach",
		])
		.output()
		.unwrap();

	let top = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "top"])
		.output()
		.unwrap();
	assert!(
		top.status.success(),
		"top failed: {}",
		String::from_utf8_lossy(&top.stderr)
	);
	// The exit code was the whole check, and passing `top.stderr` into the failure
	// message made it look like the output was being read when nothing asserted
	// anything about it. `top` exists to print processes: the container it belongs
	// to, the column header, and the command actually running inside.
	let out = String::from_utf8_lossy(&top.stdout);
	assert!(
		out.contains(&format!("{proj}-web-1")),
		"top did not name the container the processes belong to: {out:?}"
	);
	assert!(
		out.contains("PID") && out.contains("CMD"),
		"top printed no process table header: {out:?}"
	);
	assert!(
		out.contains("sleep infinity"),
		"top did not report the process running in the container: {out:?}"
	);

	Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "down"])
		.output()
		.unwrap();
}

#[tokio::test]
async fn cli_images_subcommand() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-climg", std::process::id());
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&proj,
			"up",
			"--detach",
		])
		.output()
		.unwrap();

	let images = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "images"])
		.output()
		.unwrap();
	assert!(
		images.status.success(),
		"images failed: {:?}",
		images.stderr
	);
	let stdout = String::from_utf8_lossy(&images.stdout);
	assert!(
		stdout.contains("alpine"),
		"expected 'alpine' in images output"
	);

	Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "down"])
		.output()
		.unwrap();
}
