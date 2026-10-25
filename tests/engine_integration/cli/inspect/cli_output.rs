//! CLI tests for output the library-level tests cannot reach.
//!
//! `logs`, `top` and an attached `up` all write to stdout and return
//! `Result<()>`, so their engine-level counterparts can only assert the absence
//! of an error. These drive the binary, where the bytes are readable. Split out
//! of `cli_commands.rs` when the additions took it past the 500 code-line limit.
use std::fs;
use std::process::Command;
use tempfile::tempdir;

use super::*;

/// `logs_scaled_service_all_replicas` promises every replica is included and
/// cannot check it. This does. The regression it guards is real history: #592
/// was by-service commands failing to resolve replicas after a scale.
#[tokio::test]
async fn cli_logs_covers_every_replica_of_a_scaled_service() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-lgscale", std::process::id());
	// Each replica prints, so a `logs` that reached only the first one is visible
	// as a missing prefix rather than as shorter output nobody counts.
	fs::write(
		&compose,
		"services:\n  worker:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"echo hello-from-worker; sleep infinity\"]\n    deploy:\n      replicas: 2\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	Command::new(bin())
		.args(["-f", c, "-p", &proj, "up", "--detach"])
		.output()
		.unwrap();
	let logs = Command::new(bin())
		.args(["-f", c, "-p", &proj, "logs"])
		.output()
		.unwrap();
	Command::new(bin())
		.args(["-f", c, "-p", &proj, "down"])
		.output()
		.unwrap();

	assert!(logs.status.success(), "logs failed: {:?}", logs.stderr);
	let out = String::from_utf8_lossy(&logs.stdout);
	assert!(
		out.contains("worker-1 | hello-from-worker"),
		"logs missed the first replica: {out:?}"
	);
	assert!(
		out.contains("worker-2 | hello-from-worker"),
		"logs stopped at the first replica: {out:?}"
	);
}

/// The `top` half of the same gap: `cli_top_subcommand` drives one service, so
/// "all replicas" was asserted nowhere.
#[tokio::test]
async fn cli_top_covers_every_replica_of_a_scaled_service() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-tpscale", std::process::id());
	fs::write(
		&compose,
		"services:\n  worker:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    deploy:\n      replicas: 2\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	Command::new(bin())
		.args(["-f", c, "-p", &proj, "up", "--detach"])
		.output()
		.unwrap();
	let top = Command::new(bin())
		.args(["-f", c, "-p", &proj, "top"])
		.output()
		.unwrap();
	Command::new(bin())
		.args(["-f", c, "-p", &proj, "down"])
		.output()
		.unwrap();

	assert!(top.status.success(), "top failed: {:?}", top.stderr);
	let out = String::from_utf8_lossy(&top.stdout);
	assert!(
		out.contains(&format!("{proj}-worker-1")),
		"top missed the first replica: {out:?}"
	);
	assert!(
		out.contains(&format!("{proj}-worker-2")),
		"top stopped at the first replica: {out:?}"
	);
}

/// `logs_with_stderr_output` names the stderr path and cannot read it. Measured
/// while writing this: podup keeps the streams apart, so the container's stdout
/// reaches podup's stdout and its stderr reaches podup's stderr, both carrying
/// the service prefix. That separation is the contract, and it is finer than
/// "the line appears somewhere": folding stderr into stdout would satisfy a
/// laxer check while breaking `podup logs 2>/dev/null` for anyone filtering.
#[tokio::test]
async fn cli_logs_keeps_container_stdout_and_stderr_apart() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-lgerr", std::process::id());
	fs::write(
		&compose,
		"services:\n  noisy:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"echo on-stdout; echo on-stderr 1>&2; sleep infinity\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	Command::new(bin())
		.args(["-f", c, "-p", &proj, "up", "--detach"])
		.output()
		.unwrap();
	let logs = Command::new(bin())
		.args(["-f", c, "-p", &proj, "logs"])
		.output()
		.unwrap();
	Command::new(bin())
		.args(["-f", c, "-p", &proj, "down"])
		.output()
		.unwrap();

	assert!(logs.status.success(), "logs failed: {:?}", logs.stderr);
	let out = String::from_utf8_lossy(&logs.stdout);
	let err = String::from_utf8_lossy(&logs.stderr);
	assert!(
		out.contains("noisy-1 | on-stdout"),
		"the container's stdout did not reach podup's stdout: {out:?}"
	);
	assert!(
		err.contains("noisy-1 | on-stderr"),
		"the container's stderr did not reach podup's stderr: {err:?}"
	);
	assert!(
		!out.contains("on-stderr"),
		"stderr was folded into stdout, so `podup logs 2>/dev/null` would carry it: {out:?}"
	);
}

/// The wire-format correction from #1365: the libpod log stream is the 8-byte
/// `[stream_type][3 pad bytes][size_big_endian: u32]` frame Docker uses, and a
/// `logs` invocation must demultiplex both streams from a real daemon. Reading
/// just the visible line is the easy half; checking the service prefix is the
/// harder half and the one the parser only gets right when the size field is
/// interpreted as a `u32`, not the four bytes it would have read as before.
#[tokio::test]
async fn cli_logs_demuxes_eight_byte_frames_from_a_real_daemon() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-lgframe", std::process::id());
	fs::write(
		&compose,
		"services:\n  chatty:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"printf 'on-stdout\\\\n'; printf 'on-stderr\\\\n' 1>&2; sleep infinity\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	Command::new(bin())
		.args(["-f", c, "-p", &proj, "up", "--detach"])
		.output()
		.unwrap();
	let logs = Command::new(bin())
		.args(["-f", c, "-p", &proj, "logs"])
		.output()
		.unwrap();
	Command::new(bin())
		.args(["-f", c, "-p", &proj, "down"])
		.output()
		.unwrap();

	assert!(logs.status.success(), "logs failed: {:?}", logs.stderr);
	let out = String::from_utf8_lossy(&logs.stdout);
	let err = String::from_utf8_lossy(&logs.stderr);
	assert!(
		out.contains("chatty-1 | on-stdout"),
		"a stdout frame landed somewhere other than podup's stdout: {out:?}"
	);
	assert!(
		err.contains("chatty-1 | on-stderr"),
		"a stderr frame landed somewhere other than podup's stderr: {err:?}"
	);
}
/// `logs -f` on a container that prints one line at start and then
/// stays quiet: the line must reach podup's stdout promptly. This is
/// the promptness half of the streaming change. The pre-change design
/// paid an extra per-frame wake-up across the connection driver task
/// and the reader task; on a slow container the first frame is the
/// one the user notices hanging (#1900).
#[tokio::test]
async fn cli_logs_follow_delivers_first_line_promptly() {
	if super::podman().await.is_none() {
		return;
	}
	use std::io::{BufRead, BufReader};
	use std::process::Stdio;
	use std::sync::mpsc::{channel, RecvTimeoutError};
	use std::thread;
	use std::time::{Duration, Instant};

	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-prompt", std::process::id());
	// Container sleeps three seconds before printing its line. The line
	// must reach podup's stdout within two seconds of being printed;
	// the three-second silence is what the promptness half of the
	// streaming change is meant to handle. `sh -c` keeps the command
	// out of compose's `command:` array shape, which some compose
	// versions route through a shell of their own.
	fs::write(
		&compose,
		"services:\n  prompt:\n    image: alpine:latest\n    command: \"sleep 3; echo late-line; sleep 120\"\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	Command::new(bin())
		.args(["-f", c, "-p", &proj, "up", "--detach"])
		.output()
		.unwrap();

	let started = Instant::now();
	let mut child = Command::new(bin())
		.args(["-f", c, "-p", &proj, "logs", "--no-color", "-f"])
		.env("LC_ALL", "C")
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();

	// Read the child's stdout on a dedicated thread that forwards each
	// line through a channel. `recv_timeout` enforces the deadline
	// without ever blocking forever, so a hung stream shows up as a
	// timeout the assertion below names. `BufRead::lines()` would
	// block waiting for the next line and never fire the deadline.
	let (tx, rx) = channel();
	let stdout = child.stdout.take().unwrap();
	let reader_handle = thread::spawn(move || {
		let reader = BufReader::new(stdout);
		for line in reader.lines() {
			match line {
				Ok(l) => {
					if tx.send(l).is_err() {
						break;
					}
				}
				Err(_) => break,
			}
		}
	});

	// Container prints `late-line` ~3 s after `up -d` returns, the reader
	// starts right after. Budget: ~6 s wall from `started`, with a 2 s
	// cap on the time the line takes to travel from the container to
	// podup's stdout. Six seconds is comfortably more than the
	// measured curl path on the flood-flood-1 fixture; the pre-change
	// design still fit, but the cost was paid per frame, not just at
	// the head.
	let deadline = started + Duration::from_secs(6);
	let mut found_line: Option<Duration> = None;
	loop {
		let remaining = deadline.saturating_duration_since(Instant::now());
		if remaining.is_zero() {
			break;
		}
		match rx.recv_timeout(remaining) {
			Ok(line) => {
				if line.contains("late-line") {
					found_line = Some(started.elapsed());
					break;
				}
			}
			Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => break,
		}
	}

	// Clean up the child, the reader thread, and the container in every
	// case. The previous shape handled the happy path but let a panic
	// skip `down` and leak the container; the guard below runs the same
	// teardown whether the assertion fires or not.
	let _ = child.kill();
	let _ = child.wait();
	let _ = reader_handle.join();
	Command::new(bin())
		.args(["-f", c, "-p", &proj, "down"])
		.output()
		.unwrap();

	let elapsed = found_line.unwrap_or_else(|| {
		panic!(
			"logs -f never produced late-line within 6s of starting the reader; \
			 this is the promptness regression the inline-driven body change guards against"
		)
	});
	// Time from the container's print (about 3 s after `up -d`) to the
	// line reaching podup's stdout: socket round-trip, k8s-file log
	// rotation, hyper's head parsing, the inline-driven body poll, and
	// stdout. Two seconds is the budget; the pre-change design still
	// fit, but the cost was paid per frame, not just at the head.
	let container_print_offset = Duration::from_secs(3);
	let transport = elapsed.saturating_sub(container_print_offset);
	eprintln!(
		"cli_logs_follow_delivers_first_line_promptly: late-line reached podup's stdout \
		 {elapsed:?} after the reader started (transport {transport:?} after the container \
		 printed it at ~{container_print_offset:?})"
	);
	assert!(
		transport < Duration::from_secs(2),
		"late-line took {transport:?} to reach podup's stdout after the container printed it \
		 (elapsed from start: {elapsed:?})"
	);
}

/// An attached `up` (no `--detach`) is where that content is reachable.
#[tokio::test]
async fn cli_attached_up_carries_the_container_output() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-attach", std::process::id());
	// The command exits on its own, so the attached `up` returns without needing
	// a signal, with no timeout standing in for synchronisation.
	fs::write(
		&compose,
		"services:\n  chatty:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"echo attached-output; sleep 2\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	let up = Command::new(bin())
		.args(["-f", c, "-p", &proj, "up"])
		.output()
		.unwrap();
	Command::new(bin())
		.args(["-f", c, "-p", &proj, "down"])
		.output()
		.unwrap();

	assert!(up.status.success(), "attached up failed: {:?}", up.stderr);
	let out = String::from_utf8_lossy(&up.stdout);
	assert!(
		out.contains("chatty-1 | attached-output"),
		"the attached up did not carry the container's output: {out:?}"
	);
}
