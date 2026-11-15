//! Like hostile_podman's runner: pinned environment, drained pipes, watchdog.
//! Unlike that runner, never write fixtures inside the measured invocation.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::fixture::Fixture;

pub struct Outcome {
	pub code: Option<i32>,
	pub stdout: String,
	stderr: String,
}

impl Outcome {
	pub fn text(&self) -> String {
		format!("{}\n{}", self.stdout, self.stderr)
	}
}

fn head(mut pipe: impl Read) -> String {
	let mut kept = Vec::new();
	let mut buf = [0u8; 8192];
	loop {
		let n = pipe.read(&mut buf).expect("drain child pipe");
		if n == 0 {
			break;
		}
		let take = n.min((64 * 1024usize).saturating_sub(kept.len()));
		kept.extend_from_slice(&buf[..take]);
	}
	String::from_utf8_lossy(&kept).into_owned()
}

pub fn podup(fixture: &Fixture, socket: &Path, args: &[&str]) -> Outcome {
	let mut child = Command::new("sh")
		.args(["-c", "ulimit -v 8000000; exec \"$0\" \"$@\""])
		.arg(env!("CARGO_BIN_EXE_podup"))
		.arg("--socket")
		.arg(format!("unix://{}", socket.display()))
		.args(["-f", "compose.yaml", "-p", "hostile", "--ansi", "never"])
		.args(args)
		.current_dir(&fixture.project)
		.env_clear()
		.env("PATH", "/usr/bin:/bin")
		.env("HOME", &fixture.home)
		.env("XDG_CONFIG_HOME", fixture.home.join("config"))
		.env("XDG_DATA_HOME", fixture.home.join("data"))
		.env("XDG_CACHE_HOME", fixture.home.join("cache"))
		.env("XDG_STATE_HOME", fixture.home.join("state"))
		.env("XDG_RUNTIME_DIR", fixture.home.join("run"))
		.env("TMPDIR", fixture.home.join("tmp"))
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.expect("spawn podup");
	let stdout = child.stdout.take().unwrap();
	let stderr = child.stderr.take().unwrap();
	let out = std::thread::spawn(move || head(stdout));
	let err = std::thread::spawn(move || head(stderr));
	let start = Instant::now();
	let code = loop {
		if let Some(status) = child.try_wait().expect("poll podup") {
			break status.code();
		}
		if start.elapsed() > Duration::from_secs(30) {
			child.kill().expect("kill timed-out podup");
			child.wait().expect("reap timed-out podup");
			break None;
		}
		std::thread::sleep(Duration::from_millis(15));
	};
	Outcome {
		code,
		stdout: out.join().expect("stdout reader"),
		stderr: err.join().expect("stderr reader"),
	}
}
