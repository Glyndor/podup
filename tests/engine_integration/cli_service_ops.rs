//! CLI binary integration tests for the commands that name a service: `restart`
//! with `--no-deps`, and `rm` with `-s` and `-v`.
use super::*;
use std::fs;
use tempfile::tempdir;
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

	run_ok(&["-f", c, "-p", &proj, "up", "-d"]);
	let restart = run(&["-f", c, "-p", &proj, "restart", "--no-deps", "web"]);
	assert!(
		restart.status.success(),
		"restart --no-deps failed: {:?}",
		restart.stderr
	);
	run(&["-f", c, "-p", &proj, "down"]);
}

#[tokio::test]
async fn cli_rm_volumes_removes_container() {
	if super::podman().await.is_none() {
		return;
	}
	// `up` makes the network, `stop` halts the container, `rm -v -f` removes
	// the container and any anonymous volumes it created. None of those
	// removes the project network, so the test has always left
	// `<project>_default` on the host. The guard's drop runs `down -v`,
	// which is the one CLI invocation that tears the network down too.
	let _down = super::DownGuard::new(
		"rmv",
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	);
	let c = _down.compose_path();
	let proj = _down.name();

	run_ok(&["-f", c, "-p", proj, "up", "-d"]);
	run(&["-f", c, "-p", proj, "stop"]);
	let rm = run(&["-f", c, "-p", proj, "rm", "-v", "-f"]);
	assert!(rm.status.success(), "rm -v failed: {:?}", rm.stderr);
	assert_eq!(ps_all_count(c, proj), 0, "rm must remove the container");
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

	run_ok(&["-f", c, "-p", &proj, "up", "-d"]);
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
