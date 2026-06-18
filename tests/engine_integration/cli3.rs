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

fn run(args: &[&str]) -> std::process::Output {
	Command::new(bin()).args(args).output().unwrap()
}

fn ps_all_count(compose: &str, proj: &str) -> usize {
	String::from_utf8_lossy(&run(&["-f", compose, "-p", proj, "ps", "-a", "-q"]).stdout)
		.lines()
		.filter(|l| !l.trim().is_empty())
		.count()
}

#[tokio::test]
async fn cli_down_remove_orphans_drops_undeclared_containers() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-orphan", std::process::id());
	let two = dir.path().join("two.yml");
	let one = dir.path().join("one.yml");
	let svc = "image: alpine:latest\n    command: [\"sleep\", \"infinity\"]";
	fs::write(
		&two,
		format!("services:\n  web:\n    {svc}\n  extra:\n    {svc}\n"),
	)
	.unwrap();
	fs::write(&one, format!("services:\n  web:\n    {svc}\n")).unwrap();
	let (two, one) = (two.to_str().unwrap(), one.to_str().unwrap());

	run(&["-f", two, "-p", &proj, "up", "-d"]);
	assert_eq!(ps_all_count(two, &proj), 2);

	// Down against the one-service file: --remove-orphans must also drop `extra`.
	let down = run(&["-f", one, "-p", &proj, "down", "--remove-orphans"]);
	assert!(down.status.success(), "down failed: {:?}", down.stderr);
	assert_eq!(
		ps_all_count(one, &proj),
		0,
		"--remove-orphans must remove the undeclared container too"
	);
}

#[tokio::test]
async fn cli_restart_no_deps_succeeds() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-nodeps", std::process::id());
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	run(&["-f", c, "-p", &proj, "up", "-d"]);
	let restart = run(&["-f", c, "-p", &proj, "restart", "--no-deps", "web"]);
	assert!(
		restart.status.success(),
		"restart --no-deps failed: {:?}",
		restart.stderr
	);
	run(&["-f", c, "-p", &proj, "down"]);
}

#[tokio::test]
async fn cli_up_no_build_skips_building() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-nobuild", std::process::id());
	fs::write(dir.path().join("Dockerfile"), "FROM alpine:latest\n").unwrap();
	let compose = dir.path().join("docker-compose.yml");
	// A build-only service with no prebuilt image: `--no-build` must refuse to
	// build, so `up` fails because there is no image to run.
	fs::write(&compose, "services:\n  app:\n    build: .\n").unwrap();
	let up = run(&[
		"-f",
		compose.to_str().unwrap(),
		"-p",
		&proj,
		"up",
		"-d",
		"--no-build",
	]);
	assert!(
		!up.status.success(),
		"--no-build must not build the image, so up has nothing to run"
	);
	run(&["-f", compose.to_str().unwrap(), "-p", &proj, "down"]);
}

#[tokio::test]
async fn cli_up_pull_never_starts_present_image() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-pullnever", std::process::id());
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();
	// Ensure the image is present, then `--pull never` must still start it.
	run(&["-f", c, "-p", &proj, "up", "-d"]);
	run(&["-f", c, "-p", &proj, "down"]);
	let up = run(&["-f", c, "-p", &proj, "up", "-d", "--pull", "never"]);
	assert!(
		up.status.success(),
		"up --pull never failed: {:?}",
		up.stderr
	);
	run(&["-f", c, "-p", &proj, "down"]);
}

#[tokio::test]
async fn cli_down_rmi_all_succeeds_and_removes_containers() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-rmi", std::process::id());
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	run(&["-f", c, "-p", &proj, "up", "-d"]);
	let down = run(&["-f", c, "-p", &proj, "down", "--rmi", "all"]);
	assert!(
		down.status.success(),
		"down --rmi all failed: {:?}",
		down.stderr
	);
	assert_eq!(ps_all_count(c, &proj), 0, "down must remove the containers");
}

#[tokio::test]
async fn cli_rm_volumes_removes_container() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-rmv", std::process::id());
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	run(&["-f", c, "-p", &proj, "up", "-d"]);
	run(&["-f", c, "-p", &proj, "stop"]);
	let rm = run(&["-f", c, "-p", &proj, "rm", "-v", "-f"]);
	assert!(rm.status.success(), "rm -v failed: {:?}", rm.stderr);
	assert_eq!(ps_all_count(c, &proj), 0, "rm must remove the container");
}

#[tokio::test]
async fn cli_kill_remove_orphans_drops_undeclared() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-killorph", std::process::id());
	let svc = "image: alpine:latest\n    command: [\"sleep\", \"infinity\"]";
	let two = dir.path().join("two.yml");
	let one = dir.path().join("one.yml");
	fs::write(
		&two,
		format!("services:\n  web:\n    {svc}\n  extra:\n    {svc}\n"),
	)
	.unwrap();
	fs::write(&one, format!("services:\n  web:\n    {svc}\n")).unwrap();
	let (two, one) = (two.to_str().unwrap(), one.to_str().unwrap());

	run(&["-f", two, "-p", &proj, "up", "-d"]);
	let kill = run(&["-f", one, "-p", &proj, "kill", "--remove-orphans"]);
	assert!(kill.status.success(), "kill failed: {:?}", kill.stderr);
	// The orphan `extra` is removed; the declared `web` is killed but remains.
	run(&["-f", one, "-p", &proj, "down"]);
}

#[tokio::test]
async fn cli_pull_quiet_succeeds() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-pullq", std::process::id());
	let compose = dir.path().join("docker-compose.yml");
	fs::write(&compose, "services:\n  web:\n    image: alpine:latest\n").unwrap();
	let pull = run(&["-f", compose.to_str().unwrap(), "-p", &proj, "pull", "-q"]);
	assert!(pull.status.success(), "pull -q failed: {:?}", pull.stderr);
}

#[tokio::test]
async fn cli_up_no_start_creates_without_starting() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-nostart", std::process::id());
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	let up = run(&["-f", c, "-p", &proj, "up", "--no-start"]);
	assert!(up.status.success(), "up --no-start failed: {:?}", up.stderr);
	assert_eq!(ps_all_count(c, &proj), 1, "container must be created");
	let running = String::from_utf8_lossy(&run(&["-f", c, "-p", &proj, "ps", "-q"]).stdout)
		.lines()
		.filter(|l| !l.trim().is_empty())
		.count();
	assert_eq!(running, 0, "--no-start must not start the container");
	run(&["-f", c, "-p", &proj, "down"]);
}

#[tokio::test]
async fn cli_up_wait_returns_when_healthy() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-upwait", std::process::id());
	let compose = dir.path().join("docker-compose.yml");
	// A healthcheck that omits `timeout` (defaults applied) must still reach
	// healthy, so `up --wait` returns successfully instead of timing out.
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    healthcheck:\n      test: [\"CMD\", \"true\"]\n      interval: 1s\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	let up = run(&["-f", c, "-p", &proj, "up", "-d", "--wait"]);
	assert!(up.status.success(), "up --wait failed: {:?}", up.stderr);
	run(&["-f", c, "-p", &proj, "down"]);
}

#[tokio::test]
async fn cli_rm_stop_removes_running_container() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-rmstop", std::process::id());
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	run(&["-f", c, "-p", &proj, "up", "-d"]);
	assert_eq!(ps_all_count(c, &proj), 1, "container should exist after up");

	// `rm -s` (no -f) must stop the running container first, then remove it.
	let rm = run(&["-f", c, "-p", &proj, "rm", "-s", "web"]);
	assert!(rm.status.success(), "rm -s failed: {:?}", rm.stderr);
	assert_eq!(
		ps_all_count(c, &proj),
		0,
		"rm -s must remove the running container"
	);

	run(&["-f", c, "-p", &proj, "down"]);
}

#[tokio::test]
async fn cli_start_wait_returns_after_starting() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-startwait", std::process::id());
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	// Create the container without starting, then `start --wait` must start it
	// and return (no healthcheck → ready once started).
	run(&["-f", c, "-p", &proj, "up", "--no-start"]);
	let start = run(&[
		"-f",
		c,
		"-p",
		&proj,
		"start",
		"--wait",
		"--wait-timeout",
		"30",
	]);
	assert!(
		start.status.success(),
		"start --wait failed: {:?}",
		start.stderr
	);

	run(&["-f", c, "-p", &proj, "down"]);
}
