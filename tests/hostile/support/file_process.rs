//! Isolated command execution with a deadline and sampled Linux peak RSS.
#![allow(dead_code)]

use std::fs::{self, File};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub const MARKER: &str = "OUTSIDE-H3-CANARY-45c71";
pub const CAP: usize = 16 * 1024 * 1024;

pub struct Outcome {
	pub success: bool,
	/// Exit code, `None` when the process was killed by a signal or the deadline.
	pub code: Option<i32>,
	pub timed_out: bool,
	pub text: String,
	pub rss_kib: u64,
}

pub fn command(dir: &Path, args: &[&str]) -> Command {
	let mut cmd = isolated(dir);
	cmd.args([
		"--socket",
		"unix:///nonexistent/h3.sock",
		"-p",
		"h3",
		"-f",
		"compose.yaml",
	])
	.args(args);
	cmd
}

/// `podup` run from `dir` with its home, XDG directories and the variables that
/// steer a compose load pointed at `dir` or removed, and no arguments.
pub fn isolated(dir: &Path) -> Command {
	let mut cmd = Command::new(env!("CARGO_BIN_EXE_podup"));
	cmd.current_dir(dir)
		.env("HOME", dir)
		.env("XDG_CONFIG_HOME", dir.join("config"))
		.env("XDG_DATA_HOME", dir.join("data"))
		.env("XDG_RUNTIME_DIR", dir)
		.env("NO_COLOR", "1")
		.env_remove("RUST_LOG")
		.env_remove("COMPOSE_FILE")
		.env_remove("COMPOSE_PROFILES")
		.env_remove("COMPOSE_ENV_FILES")
		.env_remove("LD_PRELOAD");
	cmd
}

struct Reap(Child);
impl Drop for Reap {
	fn drop(&mut self) {
		let _ = self.0.kill();
		let _ = self.0.wait();
	}
}

pub fn run(cmd: Command) -> Outcome {
	run_fed(cmd, None)
}

/// [`run`] with standard input written by `feed` on its own thread, so a feed
/// that never ends (an endless stream) cannot block the measurement. The feed
/// stops when podup closes its end: the write then fails.
pub fn run_fed(
	mut cmd: Command,
	feed: Option<Box<dyn FnOnce(std::process::ChildStdin) + Send>>,
) -> Outcome {
	let out = tempfile::NamedTempFile::new().unwrap();
	let err = tempfile::NamedTempFile::new().unwrap();
	let stdin = if feed.is_some() {
		Stdio::piped()
	} else {
		Stdio::null()
	};
	let mut child = Reap(
		cmd.stdin(stdin)
			.stdout(Stdio::from(File::create(out.path()).unwrap()))
			.stderr(Stdio::from(File::create(err.path()).unwrap()))
			.spawn()
			.unwrap(),
	);
	if let (Some(feed), Some(pipe)) = (feed, child.0.stdin.take()) {
		std::thread::spawn(move || feed(pipe));
	}
	let start = Instant::now();
	let mut rss_kib = 0;
	let (success, code, timed_out) = loop {
		if let Ok(status) = fs::read_to_string(format!("/proc/{}/status", child.0.id())) {
			for line in status.lines().filter(|s| s.starts_with("VmHWM:")) {
				rss_kib = rss_kib.max(line.split_whitespace().nth(1).unwrap().parse().unwrap());
			}
		}
		if let Some(status) = child.0.try_wait().unwrap() {
			break (status.success(), status.code(), false);
		}
		if start.elapsed() > Duration::from_secs(15) {
			break (false, None, true);
		}
		std::thread::sleep(Duration::from_millis(5));
	};
	drop(child);
	Outcome {
		success,
		code,
		timed_out,
		rss_kib,
		text: fs::read_to_string(out.path()).unwrap() + &fs::read_to_string(err.path()).unwrap(),
	}
}
