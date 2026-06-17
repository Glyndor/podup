//! CLI binary integration tests (covers main.rs).
use std::fs;
use std::process::Command;
use tempfile::tempdir;

use super::*;

#[tokio::test]
async fn cli_rm_subcommand() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-clrm", std::process::id());
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

	Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "stop"])
		.output()
		.unwrap();

	let rm = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "rm"])
		.output()
		.unwrap();
	assert!(rm.status.success(), "rm failed: {:?}", rm.stderr);
}

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
	assert!(stats.status.success(), "stats failed: {:?}", stats.stderr);
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

#[tokio::test]
async fn cli_up_with_build_flag() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-clbld", std::process::id());
	fs::write(dir.path().join("Dockerfile"), "FROM alpine:latest\n").unwrap();
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    build: .\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	let up = Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&proj,
			"up",
			"--detach",
			"--build",
		])
		.output()
		.unwrap();
	assert!(up.status.success(), "up --build failed: {:?}", up.stderr);

	Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "down"])
		.output()
		.unwrap();
}

#[tokio::test]
async fn cli_build_subcommand() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	fs::write(dir.path().join("Dockerfile"), "FROM alpine:latest\n").unwrap();
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    build: .\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	let build = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "build"])
		.output()
		.unwrap();
	assert!(build.status.success(), "build failed: {:?}", build.stderr);
}

#[tokio::test]
async fn cli_pause_unpause_subcommands() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-clpu", std::process::id());
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

	let pause = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "pause"])
		.output()
		.unwrap();
	assert!(pause.status.success(), "pause failed: {:?}", pause.stderr);

	let unpause = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "unpause"])
		.output()
		.unwrap();
	assert!(
		unpause.status.success(),
		"unpause failed: {:?}",
		unpause.stderr
	);

	Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "down"])
		.output()
		.unwrap();
}

#[tokio::test]
async fn cli_run_subcommand() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-clrun", std::process::id());
	fs::write(&compose, "services:\n  job:\n    image: alpine:latest\n").unwrap();

	let run = Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&proj,
			"run",
			"job",
			"echo",
			"hello",
		])
		.output()
		.unwrap();
	assert!(run.status.success(), "run failed: {:?}", run.stderr);
	let stdout = String::from_utf8_lossy(&run.stdout);
	assert!(stdout.contains("hello"), "expected 'hello' in output");
}

#[tokio::test]
async fn cli_run_nonzero_exit_propagates() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-clrxc", std::process::id());
	fs::write(&compose, "services:\n  job:\n    image: alpine:latest\n").unwrap();

	let run = Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&proj,
			"run",
			"job",
			"false",
		])
		.output()
		.unwrap();
	assert!(!run.status.success(), "expected non-zero exit from 'false'");
	assert_eq!(run.status.code(), Some(1));
}

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
	assert!(top.status.success(), "top failed: {:?}", top.stderr);

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

#[tokio::test]
async fn cli_port_subcommand() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-clprt", std::process::id());
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    ports:\n      - \"127.0.0.1:18081:80\"\n",
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

	let port = Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&proj,
			"port",
			"web",
			"80",
		])
		.output()
		.unwrap();
	assert!(port.status.success(), "port failed: {:?}", port.stderr);

	Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "down"])
		.output()
		.unwrap();
}

#[tokio::test]
async fn cli_cp_from_container() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-clcp", std::process::id());
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

	let dst = dir.path().to_str().unwrap();
	let cp = Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&proj,
			"cp",
			"web:/etc/hostname",
			dst,
		])
		.output()
		.unwrap();
	assert!(cp.status.success(), "cp failed: {:?}", cp.stderr);

	Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "down"])
		.output()
		.unwrap();
}
