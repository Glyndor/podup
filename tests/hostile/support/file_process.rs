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
	pub timed_out: bool,
	pub text: String,
	pub rss_kib: u64,
}

pub fn command(dir: &Path, args: &[&str]) -> Command {
	let mut cmd = Command::new(env!("CARGO_BIN_EXE_podup"));
	cmd.current_dir(dir)
		.args([
			"--socket",
			"unix:///nonexistent/h3.sock",
			"-p",
			"h3",
			"-f",
			"compose.yaml",
		])
		.args(args)
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

pub fn run(mut cmd: Command) -> Outcome {
	let out = tempfile::NamedTempFile::new().unwrap();
	let err = tempfile::NamedTempFile::new().unwrap();
	let mut child = Reap(
		cmd.stdin(Stdio::null())
			.stdout(Stdio::from(File::create(out.path()).unwrap()))
			.stderr(Stdio::from(File::create(err.path()).unwrap()))
			.spawn()
			.unwrap(),
	);
	let start = Instant::now();
	let mut rss_kib = 0;
	let (success, timed_out) = loop {
		if let Ok(status) = fs::read_to_string(format!("/proc/{}/status", child.0.id())) {
			for line in status.lines().filter(|s| s.starts_with("VmHWM:")) {
				rss_kib = rss_kib.max(line.split_whitespace().nth(1).unwrap().parse().unwrap());
			}
		}
		if let Some(status) = child.0.try_wait().unwrap() {
			break (status.success(), false);
		}
		if start.elapsed() > Duration::from_secs(15) {
			break (false, true);
		}
		std::thread::sleep(Duration::from_millis(5));
	};
	drop(child);
	Outcome {
		success,
		timed_out,
		rss_kib,
		text: fs::read_to_string(out.path()).unwrap() + &fs::read_to_string(err.path()).unwrap(),
	}
}
