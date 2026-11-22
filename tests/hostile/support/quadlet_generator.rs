//! Where the tests that run Podman's real Quadlet generator find it.
//!
//! The exporter writes keys that only the Quadlet of the supported floor reads.
//! A generator from an older Podman, such as the 4.9 the `ubuntu-latest` runner
//! ships, answers `unsupported key` to them, so the helper hands a test the
//! generator only when the file exists and its `-version` reaches the floor.
//! The floor is read out of the `MIN_PODMAN_VERSION` declaration, the way
//! `tests/repository/packaging/podman_floor.rs` reads it, because an
//! integration test cannot reach a `pub(crate)` item.
//!
//! A generator that is absent or too old skips the test with a `SKIP:` line on
//! stderr. Where a Podman at the floor is guaranteed (the Podman VM of the CI
//! lane), `PODUP_REQUIRE_QUADLET_GENERATOR=1` turns that skip into a failure
//! that says what was found. A version that cannot be read fails in both modes.
//! `PODUP_TEST_QUADLET_GENERATOR` replaces the search paths with one path.
//!
//! The library keeps its own copy of this logic for its unit tests, in
//! `internal/backends/quadlet/tests/generator.rs`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

type Version = (u64, u64, u64);

const SYSTEM_PATHS: [&str; 2] = ["/usr/libexec/podman/quadlet", "/usr/lib/podman/quadlet"];
const REQUIRE_VAR: &str = "PODUP_REQUIRE_QUADLET_GENERATOR";
const OVERRIDE_VAR: &str = "PODUP_TEST_QUADLET_GENERATOR";

/// The generator the tests may run, or `None` after saying why not.
pub fn generator() -> Option<PathBuf> {
	let paths = match std::env::var_os(OVERRIDE_VAR) {
		Some(path) => vec![PathBuf::from(path)],
		None => SYSTEM_PATHS.map(PathBuf::from).to_vec(),
	};
	let require = std::env::var_os(REQUIRE_VAR).is_some_and(|v| v == "1");
	resolve(&paths, floor(), require, &mut std::io::stderr().lock())
}

/// `MIN_PODMAN_VERSION` as the source declares it.
fn floor() -> Version {
	let source = include_str!("../../../internal/connection/libpod/client/mod.rs");
	source
		.lines()
		.map(str::trim_start)
		.filter(|l| !l.starts_with("//"))
		.find_map(|l| l.split_once("MIN_PODMAN_VERSION: (u64, u64, u64) = "))
		.and_then(|(_, rest)| {
			let tuple = rest
				.trim()
				.trim_end_matches(';')
				.trim()
				.strip_prefix('(')?
				.strip_suffix(')')?;
			parse_version(&tuple.replace(", ", "."))
		})
		.expect("MIN_PODMAN_VERSION is declared as a (u64, u64, u64) literal")
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
		return unavailable(&format!("no Quadlet generator at {looked}"), require, log);
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

#[cfg(test)]
mod tests {
	use super::*;
	use std::os::unix::fs::PermissionsExt;

	const FLOOR: Version = (6, 2, 0);

	fn fake(dir: &Path, output: &str, code: i32) -> PathBuf {
		let path = dir.join("quadlet");
		std::fs::write(&path, format!("#!/bin/sh\necho '{output}'\nexit {code}\n")).unwrap();
		std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
		path
	}

	fn run(paths: &[PathBuf], require: bool) -> (Option<PathBuf>, String) {
		let mut log = Vec::new();
		let found = resolve(paths, FLOOR, require, &mut log);
		(found, String::from_utf8(log).unwrap())
	}

	fn panic_message(paths: &[PathBuf], require: bool) -> String {
		let caught = std::panic::catch_unwind(|| run(paths, require)).unwrap_err();
		caught
			.downcast_ref::<String>()
			.cloned()
			.expect("the failure carries a message")
	}

	#[test]
	fn the_floor_is_read_from_the_declaration() {
		assert!(floor() >= (6, 2, 0), "{:?}", floor());
	}

	#[test]
	fn a_generator_at_or_above_the_floor_is_returned() {
		for printed in ["6.2.0", "6.10.0", "6.3.0-dev"] {
			let dir = tempfile::tempdir().unwrap();
			let path = fake(dir.path(), printed, 0);
			for require in [false, true] {
				let (found, log) = run(std::slice::from_ref(&path), require);
				assert_eq!(found, Some(path.clone()), "{printed}");
				assert_eq!(log, "", "{printed}");
			}
		}
	}

	#[test]
	fn an_old_generator_skips_without_the_switch_and_fails_with_it() {
		let dir = tempfile::tempdir().unwrap();
		let path = fake(dir.path(), "4.9.3", 0);
		let (found, log) = run(std::slice::from_ref(&path), false);
		assert_eq!(found, None);
		assert!(log.starts_with("SKIP: "), "{log}");
		assert!(log.contains("4.9.3") && log.contains("6.2.0"), "{log}");
		let message = panic_message(&[path], true);
		assert!(message.contains(REQUIRE_VAR), "{message}");
		assert!(message.contains("4.9.3"), "{message}");
	}

	#[test]
	fn an_unreadable_version_fails_even_without_the_switch() {
		for printed in ["", "quadlet", "6.2"] {
			let dir = tempfile::tempdir().unwrap();
			let path = fake(dir.path(), printed, 0);
			for require in [false, true] {
				let message = panic_message(std::slice::from_ref(&path), require);
				assert!(message.contains("not a version"), "{printed:?}: {message}");
			}
		}
	}

	#[test]
	fn an_absent_generator_skips_without_the_switch_and_fails_with_it() {
		let missing = vec![PathBuf::from("/nonexistent/podman/quadlet")];
		let (found, log) = run(&missing, false);
		assert_eq!(found, None);
		assert!(log.starts_with("SKIP: "), "{log}");
		let message = panic_message(&missing, true);
		assert!(message.contains("/nonexistent/podman/quadlet"), "{message}");
	}
}
