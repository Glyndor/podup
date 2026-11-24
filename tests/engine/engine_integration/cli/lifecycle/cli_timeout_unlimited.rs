//! CLI binary integration test for `-t -1` against Podman: the container that
//! ignores SIGTERM is not killed after any grace period, it is waited for. The
//! container ends on its own after a few seconds with status 0, where a kill
//! would leave 137, and a bound on the whole command keeps a regression from
//! hanging the run.
use super::*;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// A container that ignores SIGTERM and ends by itself, with status 0, after
/// five seconds.
const STUBBORN: &str =
	"image: alpine:latest\n    command: [\"sh\", \"-c\", \"trap '' TERM; sleep 5; exit 0\"]";

/// The bound on one command: far above what the container needs, far below a
/// hang.
const BOUND: Duration = Duration::from_secs(120);

fn exit_code_of(container: &str) -> String {
	let out = Command::new("podman")
		.args(["inspect", container, "--format", "{{.State.ExitCode}}"])
		.output()
		.expect("run podman inspect");
	String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Run `podup args...` and give back its status, killing it when it runs past
/// [`BOUND`].
fn bounded(args: &[&str]) -> std::process::Output {
	let child = Command::new(bin())
		.args(args)
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.expect("run podup");
	let pid = child.id();
	let (tx, rx) = mpsc::channel();
	std::thread::spawn(move || {
		let _ = tx.send(child.wait_with_output());
	});
	match rx.recv_timeout(BOUND) {
		Ok(out) => out.expect("podup output"),
		Err(_) => {
			// SAFETY: a plain signal to a child this test started.
			unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
			panic!("podup {args:?} did not finish within {BOUND:?}");
		}
	}
}

#[tokio::test]
async fn cli_stop_with_minus_one_waits_for_a_container_that_ignores_sigterm() {
	if super::podman().await.is_none() {
		return;
	}
	let guard = super::DownGuard::new("tminus", &format!("services:\n  web:\n    {STUBBORN}\n"));
	let (c, proj) = (guard.compose_path(), guard.name());
	let web = format!("{proj}-web-1");

	run_ok(&["-f", c, "-p", proj, "up", "-d"]);
	let out = bounded(&["-f", c, "-p", proj, "stop", "-t", "-1", "web"]);
	assert!(
		out.status.success(),
		"stop -t -1 failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	assert_eq!(
		exit_code_of(&web),
		"0",
		"-t -1 waits for the container to end by itself: no kill, so no 137"
	);
}

#[tokio::test]
async fn cli_stop_with_a_short_timeout_kills_the_same_container() {
	if super::podman().await.is_none() {
		return;
	}
	let guard = super::DownGuard::new("tshort", &format!("services:\n  web:\n    {STUBBORN}\n"));
	let (c, proj) = (guard.compose_path(), guard.name());
	let web = format!("{proj}-web-1");

	run_ok(&["-f", c, "-p", proj, "up", "-d"]);
	let out = bounded(&["-f", c, "-p", proj, "stop", "-t", "1", "web"]);
	assert!(
		out.status.success(),
		"stop -t 1 failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	assert_eq!(
		exit_code_of(&web),
		"137",
		"the control: the same container is killed after the one-second grace"
	);
}

#[tokio::test]
async fn cli_down_and_restart_accept_minus_one_too() {
	if super::podman().await.is_none() {
		return;
	}
	let guard = super::DownGuard::new("tminusdr", &format!("services:\n  web:\n    {STUBBORN}\n"));
	let (c, proj) = (guard.compose_path(), guard.name());

	run_ok(&["-f", c, "-p", proj, "up", "-d"]);
	let restart = bounded(&["-f", c, "-p", proj, "restart", "-t", "-1", "web"]);
	assert!(
		restart.status.success(),
		"restart -t -1 failed: {}",
		String::from_utf8_lossy(&restart.stderr)
	);
	let down = bounded(&["-f", c, "-p", proj, "down", "-t", "-1"]);
	assert!(
		down.status.success(),
		"down -t -1 failed: {}",
		String::from_utf8_lossy(&down.stderr)
	);
	assert_eq!(ps_all_count(c, proj), 0, "down removes the container");
}
