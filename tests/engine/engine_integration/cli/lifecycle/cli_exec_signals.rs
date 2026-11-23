//! A signal sent to `podup exec` reaches the command it runs.
//!
//! The command traps TERM, INT and HUP and exits with 42. Before the relay, the
//! signal ended podup with 143 or 130 and the command went on running in the
//! container with nobody reading its output (`docker compose exec` does the
//! same, measured on the same Podman 6.2.0 socket).
//!
//! Every wait in here has a limit and fails with a message: a signal that is
//! lost, or a command that never ends, must be a red test and not a process
//! that sits for hours in CI.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use super::*;

const TRAP: &str =
	"trap 'echo got-signal; kill $!; exit 42' TERM INT HUP; echo ready; sleep 100 & wait";

/// How long a podup command that is expected to answer promptly may take.
const COMMAND_LIMIT: Duration = Duration::from_secs(90);
/// How long the command may take to say it is ready, and to end after a signal.
const READY_LIMIT: Duration = Duration::from_secs(30);
const END_LIMIT: Duration = Duration::from_secs(20);

/// Run `podup` with a limit; kill it and fail if it does not end in time.
fn run_limited(args: &[&str]) -> Output {
	let mut child = Command::new(bin())
		.args(args)
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	let started = Instant::now();
	loop {
		if child.try_wait().unwrap().is_some() {
			return child.wait_with_output().unwrap();
		}
		if started.elapsed() > COMMAND_LIMIT {
			let _ = child.kill();
			let _ = child.wait();
			panic!("podup {args:?} did not end in {COMMAND_LIMIT:?}");
		}
		std::thread::sleep(Duration::from_millis(50));
	}
}

/// A one-service project that is taken down when the test ends, however it ends.
struct Project {
	_dir: tempfile::TempDir,
	file: String,
	name: String,
}

impl Project {
	fn up(tag: &str) -> Self {
		let dir = tempfile::tempdir().unwrap();
		let compose = dir.path().join("docker-compose.yml");
		fs::write(
			&compose,
			"services:\n  s:\n    image: busybox:latest\n    command: [\"sleep\", \"300\"]\n",
		)
		.unwrap();
		let project = Self {
			file: compose.to_str().unwrap().to_string(),
			name: format!("t{}-{tag}", std::process::id()),
			_dir: dir,
		};
		let up = run_limited(&["-f", &project.file, "-p", &project.name, "up", "-d"]);
		assert!(
			up.status.success(),
			"{}",
			String::from_utf8_lossy(&up.stderr)
		);
		project
	}

	fn args<'a>(&'a self, rest: &[&'a str]) -> Vec<&'a str> {
		let mut all = vec!["-f", &self.file, "-p", &self.name];
		all.extend_from_slice(rest);
		all
	}
}

impl Drop for Project {
	fn drop(&mut self) {
		let _ = Command::new(bin())
			.args(self.args(&["down", "-t", "0"]))
			.stdin(Stdio::null())
			.output();
	}
}

/// The lines a child writes to stdout, read on a thread so that waiting for them
/// can have a limit.
fn lines_of(child: &mut Child) -> Receiver<String> {
	let (tx, rx) = mpsc::channel();
	let out = child.stdout.take().unwrap();
	std::thread::spawn(move || {
		for line in BufReader::new(out).lines().map_while(Result::ok) {
			if tx.send(line).is_err() {
				return;
			}
		}
	});
	rx
}

fn send(child: &Child, signal: &str) {
	let sent = Command::new("kill")
		.args([&format!("-{signal}"), &child.id().to_string()])
		.status()
		.unwrap();
	assert!(sent.success());
}

/// Wait for the child to end; kill it and fail with `what` if it does not.
fn finish(child: &mut Child, what: &str) -> std::process::ExitStatus {
	let started = Instant::now();
	loop {
		if let Some(status) = child.try_wait().unwrap() {
			return status;
		}
		if started.elapsed() > END_LIMIT {
			let _ = child.kill();
			let _ = child.wait();
			panic!("podup exec did not end within {END_LIMIT:?} of {what}");
		}
		std::thread::sleep(Duration::from_millis(50));
	}
}

fn check(tag: &str, signal: &str, stdin: fn() -> Stdio) {
	let project = Project::up(tag);
	let mut child = Command::new(bin())
		.args(project.args(&["exec", "-T", "s", "sh", "-c", TRAP]))
		.stdin(stdin())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	let lines = lines_of(&mut child);
	match lines.recv_timeout(READY_LIMIT) {
		Ok(line) => assert_eq!(line, "ready"),
		Err(_) => {
			let _ = child.kill();
			let _ = child.wait();
			panic!("the command did not say it was ready within {READY_LIMIT:?}");
		}
	}

	send(&child, signal);
	let status = finish(&mut child, &format!("SIG{signal}"));
	let mut err = String::new();
	child
		.stderr
		.take()
		.unwrap()
		.read_to_string(&mut err)
		.unwrap();
	let rest: Vec<String> = lines.try_iter().collect();

	// What the command left running in the container.
	let leftover = run_limited(&project.args(&["exec", "-T", "s", "ps"]));
	assert!(
		rest.iter().any(|l| l == "got-signal"),
		"the command never saw {signal}: {rest:?} {err}"
	);
	assert_eq!(status.code(), Some(42), "{err}");
	assert!(
		!String::from_utf8_lossy(&leftover.stdout).contains("sleep 100"),
		"the command is still running: {}",
		String::from_utf8_lossy(&leftover.stdout)
	);
}

#[tokio::test]
async fn sigterm_reaches_the_command_and_its_status_is_returned() {
	if super::podman().await.is_none() {
		return;
	}
	check("exsigterm", "TERM", Stdio::null);
}

#[tokio::test]
async fn sigint_reaches_the_command_and_its_status_is_returned() {
	if super::podman().await.is_none() {
		return;
	}
	check("exsigint", "INT", Stdio::null);
}

/// The same when the caller's stdin is attached, a different path in podup.
#[tokio::test]
async fn the_signal_also_reaches_a_command_that_has_stdin_attached() {
	if super::podman().await.is_none() {
		return;
	}
	check("exsigpipe", "TERM", Stdio::piped);
}
