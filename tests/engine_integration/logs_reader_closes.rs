//! `podup logs -f` under a reader that stops early.
use std::fs;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use tempfile::tempdir;

use super::*;

/// A `logs -f` reader that closes its pipe after one line ends podup
/// cleanly: the next write fails with `BrokenPipe`, the follow loop stops
/// (`stop_on_write_error` in `engine/query/mod.rs`, #1102), the streaming
/// body is dropped with its connection, and the process exits 0.
///
/// The container prints a line every 100 ms and never stops, so the next
/// write after the close comes within a tick. A follow loop that ignored
/// the failed write, or a dropped body that kept the process alive, would
/// leave podup running until the deadline below kills it.
#[tokio::test]
async fn cli_logs_follow_exits_when_reader_closes() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-logspipe", std::process::id());
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"while true; do echo tick; sleep 0.1; done\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	Command::new(bin())
		.args(["-f", c, "-p", &proj, "up", "-d"])
		.output()
		.unwrap();

	let mut child = Command::new(bin())
		.args(["-f", c, "-p", &proj, "logs", "-f", "--no-color"])
		.stdout(Stdio::piped())
		.stderr(Stdio::null())
		.spawn()
		.unwrap();

	// Read the first line on a thread so a podup that never prints cannot
	// hang the test. The thread owns the only read end of the pipe and drops
	// it as soon as it returns, which is the close podup must notice.
	let stdout = child.stdout.take().unwrap();
	let (tx, rx) = channel();
	std::thread::spawn(move || {
		let mut line = String::new();
		let read = BufReader::new(stdout).read_line(&mut line);
		let _ = tx.send(read.map(|_| line));
	});
	let first = rx.recv_timeout(Duration::from_secs(15));

	let closed = Instant::now();
	let status = loop {
		if let Some(status) = child.try_wait().unwrap() {
			break Some(status);
		}
		if closed.elapsed() > Duration::from_secs(5) {
			let _ = child.kill();
			let _ = child.wait();
			break None;
		}
		std::thread::sleep(Duration::from_millis(50));
	};
	let exit_after = closed.elapsed();

	Command::new(bin())
		.args(["-f", c, "-p", &proj, "down", "-t", "0"])
		.output()
		.unwrap();

	let first = first
		.expect("podup logs -f printed no line within 15 s")
		.expect("reading podup's stdout failed");
	assert!(
		first.contains("tick"),
		"the first followed line must be the container's output, got {first:?}"
	);
	let status = status.unwrap_or_else(|| {
		panic!("podup logs -f was still running {exit_after:?} after its reader closed the pipe")
	});
	assert!(
		status.success(),
		"podup must exit 0 when its reader closes the pipe, got {status:?}"
	);
}
