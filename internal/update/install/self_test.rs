use std::path::Path;

use crate::ComposeError;

/// How long to keep retrying a spawn that reports ETXTBSY.
///
/// The window is short by nature (it lasts only as long as some other process
/// holds a write descriptor to the file across its own exec) so a second is
/// generous. Bounded on purpose: a binary that is genuinely unrunnable must
/// reach the rollback, not wedge the updater retrying forever.
#[cfg(unix)]
const TEXT_FILE_BUSY_BUDGET: std::time::Duration = std::time::Duration::from_secs(1);

/// Spawn `target --version`, retrying while the kernel says the file is still
/// open for writing somewhere.
///
/// ETXTBSY is not a property of the binary: it means another process holds a
/// write descriptor to it across its own `exec`, and `O_CLOEXEC` does not close
/// that window. Treating it as a failed self-test rolls back a signed, verified,
/// perfectly good update over a race that resolves in milliseconds, and tells
/// the user their new version is broken.
pub(crate) fn spawn_version_probe(target: &Path) -> std::io::Result<std::process::Child> {
	use std::process::{Command, Stdio};

	let probe = || {
		Command::new(target)
			.arg("--version")
			.stdin(Stdio::null())
			.stdout(Stdio::piped())
			.stderr(Stdio::null())
			.spawn()
	};

	#[cfg(unix)]
	{
		let deadline = std::time::Instant::now() + TEXT_FILE_BUSY_BUDGET;
		loop {
			match probe() {
				Err(e) if super::is_text_file_busy(&e) && std::time::Instant::now() < deadline => {
					std::thread::sleep(std::time::Duration::from_millis(10));
				}
				other => return other,
			}
		}
	}
	#[cfg(not(unix))]
	{
		probe()
	}
}

/// Whether a spawn error is ETXTBSY, asked of the `io::Error` itself.
///
/// This has to happen before the error is formatted into a message: once it is
/// a `String` the errno is gone, and matching on the text would break under any
/// locale or libc that words it differently.
#[cfg(unix)]
pub(crate) fn is_text_file_busy(e: &std::io::Error) -> bool {
	// `ExecutableFileBusy` is the named form; the raw errno is compared too so
	// this holds on a toolchain where the mapping differs.
	e.kind() == std::io::ErrorKind::ExecutableFileBusy || e.raw_os_error() == Some(libc::ETXTBSY)
}

/// Confirm a freshly-installed binary runs and reports `expected_version` by
/// invoking `--version`, bounded by a timeout so a hung binary can't wedge the
/// updater. The version check closes the rollback window: a replayed older
/// (signed) release fails here and is rolled back.
pub(crate) fn self_test(target: &Path, expected_version: &str) -> crate::Result<()> {
	use std::time::{Duration, Instant};

	let mut child = super::spawn_version_probe(target)
		.map_err(|e| ComposeError::Update(format!("could not run the updated binary: {e}")))?;

	let deadline = Instant::now() + Duration::from_secs(10);
	let answer = read_first_line(child.stdout.take());
	let status = loop {
		match child.try_wait() {
			Ok(Some(status)) => break status,
			Ok(None) => {
				if Instant::now() >= deadline {
					let _ = child.kill();
					let _ = child.wait();
					return Err(ComposeError::Update(
						"updated binary did not respond to --version within 10s".to_string(),
					));
				}
				std::thread::sleep(Duration::from_millis(50));
			}
			Err(e) => {
				return Err(ComposeError::Update(format!(
					"waiting on the updated binary failed: {e}"
				)))
			}
		}
	};
	if !status.success() {
		return Err(ComposeError::Update(format!(
			"updated binary exited with {status} on --version"
		)));
	}
	// The line is read by a thread of its own, so a descendant that outlived
	// the binary and still holds the pipe open cannot keep the updater waiting
	// past the deadline: the answer is the first line, not the end of the pipe.
	let out = answer
		.recv_timeout(deadline.saturating_duration_since(Instant::now()))
		.map_err(|_| {
			ComposeError::Update("updated binary did not report its version within 10s".to_string())
		})?;
	let reported_matches = out
		.split_whitespace()
		.any(|t| t == expected_version || t.trim_start_matches('v') == expected_version);
	if !reported_matches {
		return Err(ComposeError::Update(format!(
			"updated binary reports {:?} instead of the resolved release version \
			 {expected_version}; possible release-metadata tampering (rollback)",
			out.trim()
		)));
	}
	Ok(())
}

/// The most `--version` output the self-test will read.
const VERSION_OUTPUT_CAP: u64 = 4096;

/// Read the first line of the probe's stdout (at most [`VERSION_OUTPUT_CAP`]
/// bytes) on a helper thread and hand it back over a channel.
///
/// The thread ends when it has a line, at end of file, or at the cap. If the
/// binary writes nothing and a descendant keeps the pipe open the thread stays
/// blocked on the read, but only until that descendant exits; the caller has
/// long since given up by then and the process does not wait for it.
fn read_first_line(stdout: Option<std::process::ChildStdout>) -> std::sync::mpsc::Receiver<String> {
	use std::io::{BufRead, BufReader, Read};

	let (tx, rx) = std::sync::mpsc::channel();
	std::thread::spawn(move || {
		let mut line = String::new();
		if let Some(stdout) = stdout {
			let mut bytes = Vec::new();
			let _ = BufReader::new(stdout.take(VERSION_OUTPUT_CAP)).read_until(b'\n', &mut bytes);
			line = String::from_utf8_lossy(&bytes).into_owned();
		}
		let _ = tx.send(line);
	});
	rx
}
