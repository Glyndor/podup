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
	use std::io::Read;
	use std::time::{Duration, Instant};

	let mut child = super::spawn_version_probe(target)
		.map_err(|e| ComposeError::Update(format!("could not run the updated binary: {e}")))?;

	let deadline = Instant::now() + Duration::from_secs(10);
	let status = loop {
		match child.try_wait() {
			Ok(Some(status)) => break status,
			Ok(None) => {
				if Instant::now() >= deadline {
					let _ = child.kill();
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
	// `--version` output is a single short line; reading after exit is safe
	// (it fits the pipe buffer, so the child never blocks on a full pipe).
	let mut out = String::new();
	if let Some(mut stdout) = child.stdout.take() {
		let _ = stdout.read_to_string(&mut out);
	}
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
