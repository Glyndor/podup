//! Run the `podup` binary against a fake socket and watch what it does.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

/// Address-space ceiling for the child, in KiB: the 8 GB the task asked for.
/// A client that buffers without bound dies on it instead of taking the host.
const ADDRESS_SPACE_KIB: u32 = 8_000_000;

/// Peak resident memory the child may reach while a hostile peer feeds it.
pub const RSS_CEILING_KIB: u64 = 300 * 1024;

pub struct Outcome {
	/// `None` when the child was killed: by the budget, or by a signal of its own.
	pub code: Option<i32>,
	pub text: String,
	pub took: Duration,
	/// Highest resident set seen while it ran, in KiB.
	pub peak_kib: u64,
}

fn peak_rss_kib(pid: u32) -> u64 {
	let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
		return 0;
	};
	status
		.lines()
		.find_map(|l| l.strip_prefix("VmHWM:"))
		.and_then(|v| v.split_whitespace().next()?.parse().ok())
		.unwrap_or(0)
}

/// Read a pipe to its end, keeping only the first 64 KiB: a flood of output
/// must not become a flood of memory in the test.
fn head(pipe: &mut impl Read) -> String {
	let (mut kept, mut buf) = (Vec::new(), [0u8; 8192]);
	while let Ok(n) = pipe.read(&mut buf) {
		if n == 0 {
			break;
		}
		if kept.len() < 64 * 1024 {
			kept.extend_from_slice(&buf[..n]);
		}
	}
	String::from_utf8_lossy(&kept).into_owned()
}

/// Run `podup --socket unix://<sock> ...` against a fake daemon.
pub fn podup(sock: &Path, home: &TempDir, args: &[&str], budget: Duration) -> Outcome {
	let socket = format!("unix://{}", sock.display());
	podup_raw(Some(&socket), &[], home, args, budget)
}

/// Run `podup [--socket <socket>] -f compose.yaml -p hostile <args>` inside
/// `home`, with `env` added.
///
/// Everything the child could read from the environment is pinned to `home`,
/// so a developer's own Podman, registry login or config never takes part.
pub fn podup_raw(
	socket: Option<&str>,
	env: &[(&str, &str)],
	home: &TempDir,
	args: &[&str],
	budget: Duration,
) -> Outcome {
	let compose = home.path().join("compose.yaml");
	std::fs::write(&compose, "services:\n  web:\n    image: alpine\n").expect("write compose");
	let script = format!("ulimit -v {ADDRESS_SPACE_KIB}; exec \"$0\" \"$@\"");
	let mut child = Command::new("sh")
		.arg("-c")
		.arg(script)
		.arg(env!("CARGO_BIN_EXE_podup"))
		.args(socket.iter().flat_map(|s| ["--socket", s]))
		.args(["-f", compose.to_str().unwrap(), "-p", "hostile"])
		.args(args)
		.env_clear()
		.env("PATH", "/usr/bin:/bin")
		.env("HOME", home.path())
		.env("XDG_CONFIG_HOME", home.path().join("config"))
		.env("XDG_DATA_HOME", home.path().join("data"))
		.env("XDG_RUNTIME_DIR", home.path().join("run"))
		.envs(env.iter().copied())
		.current_dir(home.path())
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.expect("spawn podup");
	let pid = child.id();
	let (mut out, mut err) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
	let drain = std::thread::spawn(move || {
		let err_head = std::thread::spawn(move || head(&mut err));
		let out_head = head(&mut out);
		format!("{out_head}{}", err_head.join().unwrap_or_default())
	});
	let start = Instant::now();
	let mut peak_kib = 0;
	let status = loop {
		peak_kib = peak_kib.max(peak_rss_kib(pid));
		if let Some(status) = child.try_wait().expect("poll podup") {
			break Some(status);
		}
		if start.elapsed() > budget {
			let _ = child.kill();
			let _ = child.wait();
			break None;
		}
		std::thread::sleep(Duration::from_millis(15));
	};
	Outcome {
		code: status.and_then(|s| s.code()),
		text: drain.join().expect("join reader"),
		took: start.elapsed(),
		peak_kib,
	}
}

/// The process ended by itself, did not panic, and stayed under the memory
/// ceiling. It may have exited 0: some hostile bodies are simply ignored.
pub fn assert_survived(what: &str, o: &Outcome) {
	assert_ne!(
		o.code, None,
		"{what}: killed (a hang past the budget or a signal) after {:?}\n{}",
		o.took, o.text
	);
	assert!(
		!o.text.contains("panicked at"),
		"{what}: panicked\n{}",
		o.text
	);
	assert!(
		o.peak_kib < RSS_CEILING_KIB,
		"{what}: peak resident memory {} KiB passed the {} KiB ceiling",
		o.peak_kib,
		RSS_CEILING_KIB
	);
}

/// [`assert_survived`], and the exit code is a failure: the hostile reply was
/// refused, not taken as an answer.
pub fn assert_refused(what: &str, o: &Outcome) {
	assert_survived(what, o);
	assert_ne!(
		o.code,
		Some(0),
		"{what}: exited 0 on hostile input\n{}",
		o.text
	);
}
