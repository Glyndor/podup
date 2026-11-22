//! Where the tests that run Podman's real Quadlet generator find it.
//!
//! The renderer writes keys that only the Quadlet of the supported floor reads
//! (`MIN_PODMAN_VERSION`). A generator from an older Podman, such as the 4.9
//! the `ubuntu-latest` runner ships, answers `unsupported key` to them, so a
//! test that ran it would fail for a reason that is no fault of the code. The
//! helper therefore hands a test the generator only when the file exists and
//! its `-version` reaches the floor.
//!
//! A generator that is absent or too old skips the test with a `SKIP:` line on
//! stderr. Where a Podman at the floor is guaranteed (the Podman VM of the CI
//! lane), `PODUP_REQUIRE_QUADLET_GENERATOR=1` turns that skip into a failure
//! that says what was found, because a skip reports `ok`. A version that cannot
//! be read fails in both modes: nothing says the generator is old, so skipping
//! would hide a generator that is not.
//!
//! `PODUP_TEST_QUADLET_GENERATOR` replaces the search paths with one path. It
//! exists to run the real tests against a stand-in without touching the system.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::libpod::client::MIN_PODMAN_VERSION;

type Version = (u64, u64, u64);

const SYSTEM_PATHS: [&str; 2] = ["/usr/libexec/podman/quadlet", "/usr/lib/podman/quadlet"];
const REQUIRE_VAR: &str = "PODUP_REQUIRE_QUADLET_GENERATOR";
const OVERRIDE_VAR: &str = "PODUP_TEST_QUADLET_GENERATOR";

/// The generator these tests may run, or `None` after saying why not.
pub(in crate::backends::quadlet) fn generator() -> Option<PathBuf> {
	match std::env::var_os(OVERRIDE_VAR) {
		Some(path) => generator_from(&[PathBuf::from(path)]),
		None => generator_from(&SYSTEM_PATHS.map(PathBuf::from)),
	}
}

/// [`generator`] over an explicit list of paths: the first one that is a file
/// is the candidate.
pub(in crate::backends::quadlet) fn generator_from(paths: &[PathBuf]) -> Option<PathBuf> {
	let require = std::env::var_os(REQUIRE_VAR).is_some_and(|v| v == "1");
	resolve(
		paths,
		MIN_PODMAN_VERSION,
		require,
		&mut std::io::stderr().lock(),
	)
}

/// The policy, with the floor, the switch and the skip message injected.
/// Panics where the test must fail; returns `None` where it skips.
fn resolve(
	paths: &[PathBuf],
	floor: Version,
	require: bool,
	log: &mut dyn Write,
) -> Option<PathBuf> {
	let floor_text = format!("{}.{}.{}", floor.0, floor.1, floor.2);
	let Some(path) = paths.iter().find(|p| p.is_file()) else {
		let looked = paths
			.iter()
			.map(|p| p.display().to_string())
			.collect::<Vec<_>>()
			.join(", ");
		let why = format!("no Quadlet generator at {looked}");
		return unavailable(&why, require, log);
	};
	let text = match version_text(path) {
		Ok(text) => text,
		Err(e) => panic!("cannot run {} -version: {e}", path.display()),
	};
	let Some(found) = parse_version(&text) else {
		panic!(
			"{} -version printed {text:?}, which is not a version; \
			 the generator cannot be judged against {floor_text}",
			path.display()
		);
	};
	if found < floor {
		let why = format!(
			"{} is version {text:?}, below the {floor_text} the exporter writes for",
			path.display()
		);
		return unavailable(&why, require, log);
	}
	Some(path.clone())
}

fn unavailable(why: &str, require: bool, log: &mut dyn Write) -> Option<PathBuf> {
	assert!(
		!require,
		"{REQUIRE_VAR}=1 but a test cannot run the generator: {why}"
	);
	let _ = writeln!(
		log,
		"SKIP: {why}; the generator round-trip was not executed"
	);
	None
}

/// What `<generator> -version` prints. A script written a moment ago can still
/// be held open for writing by a fork of another test thread; the kernel then
/// answers `ETXTBSY` until that child execs, so that one error is retried.
fn version_text(path: &Path) -> std::io::Result<String> {
	let mut tries = 0;
	loop {
		match Command::new(path).arg("-version").output() {
			Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 50 => {
				tries += 1;
				std::thread::sleep(std::time::Duration::from_millis(20));
			}
			Err(e) => return Err(e),
			Ok(out) if !out.status.success() => {
				return Err(std::io::Error::other(format!(
					"exit {}: {}",
					out.status,
					String::from_utf8_lossy(&out.stderr).trim()
				)));
			}
			Ok(out) => return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string()),
		}
	}
}

/// `6.2.0`, `v6.2.0` or `6.3.0-dev` as three numbers; anything else is not a
/// version.
fn parse_version(text: &str) -> Option<Version> {
	let text = text.strip_prefix('v').unwrap_or(text);
	let core = text.split(['-', '+']).next()?;
	let mut parts = core.split('.');
	let version = (
		parts.next()?.parse().ok()?,
		parts.next()?.parse().ok()?,
		parts.next()?.parse().ok()?,
	);
	parts.next().is_none().then_some(version)
}

#[cfg(all(test, unix))]
#[path = "generator_tests.rs"]
mod tests;
