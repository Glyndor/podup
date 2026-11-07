//! Every lockfile that records podup's own version records the one in `Cargo.toml`.
//!
//! `fuzz` links podup by path, so its `Cargo.lock` carries a `podup` package with
//! the crate's version, and it is outside the workspace, so a release bump of the
//! root lock does not touch it. Nothing noticed: `release.yml` checks the root
//! lock only and `cargo fuzz build` does not pass `--locked`, so cargo rewrote the
//! stale line on every build and the file sat at 5.15.1 through four releases
//! while the crate went to 5.16.0. A lock that `--locked` would refuse is a lock
//! the next maintainer finds as an unexplained diff.

use std::fs;
use std::path::Path;

fn repo(rel: &str) -> String {
	let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
	fs::read_to_string(&path).unwrap_or_else(|e| panic!("{rel} is readable: {e}"))
}

/// The `version` line right after `name = "podup"`, which is the crate itself and
/// not a dependency that shares the number.
fn podup_version_in_lock(lock: &str) -> Option<String> {
	let mut lines = lock.lines();
	while let Some(line) = lines.next() {
		if line == "name = \"podup\"" {
			let version = lines.next()?.strip_prefix("version = \"")?;
			return Some(version.trim_end_matches('"').to_string());
		}
	}
	None
}

fn crate_version(manifest: &str) -> String {
	let line = manifest
		.lines()
		.find(|l| l.starts_with("version = "))
		.expect("Cargo.toml has a version line");
	line.split('"')
		.nth(1)
		.expect("a quoted version")
		.to_string()
}

/// One line per lock whose podup version differs from `expected`.
fn disagreements(expected: &str, locks: &[(&str, String)]) -> Vec<String> {
	locks
		.iter()
		.filter_map(|(name, text)| {
			let found = podup_version_in_lock(text)?;
			(found != expected)
				.then(|| format!("{name} records podup {found}, Cargo.toml is at {expected}"))
		})
		.collect()
}

#[test]
fn the_lockfiles_record_the_crate_version() {
	let expected = crate_version(&repo("Cargo.toml"));
	let mut locks = vec![
		("Cargo.lock", repo("Cargo.lock")),
		("fuzz/Cargo.lock", repo("fuzz/Cargo.lock")),
	];
	// `bench/timeit` does not link podup today, so its lock has no such package
	// and is skipped by the comparison; if it ever does, it joins. It is read
	// only when present because a source package may leave `bench/` out.
	if let Ok(text) =
		fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("bench/timeit/Cargo.lock"))
	{
		locks.push(("bench/timeit/Cargo.lock", text));
	}
	// A reader that found nothing would agree with everything. The root lock and
	// the fuzz lock both carry the package, so both must have been read.
	for (name, text) in &locks[..2] {
		assert!(
			podup_version_in_lock(text).is_some(),
			"{name} has no podup package, so this test would check nothing"
		);
	}
	let problems = disagreements(&expected, &locks);
	assert!(
		problems.is_empty(),
		"\n{}\nrun `cargo update -p podup` in the root and in fuzz/\n",
		problems.join("\n")
	);
}

#[test]
fn a_stale_lock_is_reported() {
	let stale = "[[package]]\nname = \"podup\"\nversion = \"5.15.1\"\n".to_string();
	let found = disagreements("5.16.0", &[("fuzz/Cargo.lock", stale)]);
	assert_eq!(
		found,
		["fuzz/Cargo.lock records podup 5.15.1, Cargo.toml is at 5.16.0"]
	);
}

#[test]
fn a_dependency_with_the_same_number_is_not_the_crate() {
	let lock = "[[package]]\nname = \"podup-fuzz\"\nversion = \"9.9.9\"\n\n\
		[[package]]\nname = \"podup\"\nversion = \"5.16.0\"\n";
	assert_eq!(podup_version_in_lock(lock).as_deref(), Some("5.16.0"));
	assert!(disagreements("5.16.0", &[("l", lock.to_string())]).is_empty());
}
