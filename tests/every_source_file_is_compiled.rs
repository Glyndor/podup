//! Every `.rs` file under `internal/` and `tests/` is compiled.
//!
//! Cargo compiles only the files a `mod` declaration reaches from the crate
//! roots, and says nothing about the rest. A test file whose declaration is
//! lost never compiles again, so it cannot fail, and the test count does not
//! drop because it never counted. #1666 moved code out of
//! `engine/container/mod.rs` and took `mod spec_body_tests;` with it; the test
//! that pinned every `SpecGenerator` field then did not run for three weeks
//! (#1926). Two new test files in #1900 were never declared at all.
//!
//! This walks the module tree the way rustc resolves it, from `lib.rs` and
//! `main.rs` and from each `tests/*.rs`, and fails on any file under
//! `internal/` or `tests/` it does not reach.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The files a `mod name;` declaration in `file` can load, in rustc's order.
///
/// `#[path = "…"]` is relative to the declaring file's directory. Without it,
/// a crate root or a `mod.rs` declares its children beside itself, and any
/// other `foo.rs` declares them in `foo/`.
fn declared_files(file: &Path, is_root: bool, name: &str, path_attr: Option<&str>) -> Vec<PathBuf> {
	let dir = file.parent().expect("a source file has a directory");
	if let Some(rel) = path_attr {
		return vec![dir.join(rel)];
	}
	let stem = file.file_stem().unwrap().to_string_lossy();
	let base = if is_root || stem == "mod" {
		dir.to_path_buf()
	} else {
		dir.join(stem.as_ref())
	};
	vec![
		base.join(format!("{name}.rs")),
		base.join(name).join("mod.rs"),
	]
}

/// The `mod name;` declarations in `src`, each with the `#[path]` attribute
/// that precedes it, if any. Attributes such as `#[cfg(test)]` may sit between
/// the two; comments and blank lines may too.
fn mod_declarations(src: &str) -> Vec<(String, Option<String>)> {
	let mut out = Vec::new();
	let mut pending_path: Option<String> = None;
	for line in src.lines() {
		let t = line.trim();
		if let Some(rest) = t.strip_prefix("#[path = \"") {
			pending_path = rest.split('"').next().map(str::to_string);
			continue;
		}
		if t.starts_with("#[") || t.starts_with("//") || t.is_empty() {
			continue;
		}
		// Any visibility: `pub`, `pub(crate)`, `pub(super)`, `pub(in crate::x)`.
		let decl = match t.strip_prefix("pub") {
			Some(rest) if rest.starts_with('(') => rest
				.split_once(')')
				.map_or(t, |(_, after)| after.trim_start()),
			Some(rest) if rest.starts_with(' ') => rest.trim_start(),
			_ => t,
		};
		if let Some(name) = decl
			.strip_prefix("mod ")
			.and_then(|r| r.strip_suffix(';'))
			.map(str::trim)
		{
			out.push((name.to_string(), pending_path.take()));
		} else {
			pending_path = None;
		}
	}
	out
}

fn all_sources(dir: &Path, out: &mut BTreeSet<PathBuf>) {
	for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
		let path = entry.unwrap().path();
		if path.is_dir() {
			all_sources(&path, out);
		} else if path.extension().is_some_and(|e| e == "rs") {
			out.insert(path);
		}
	}
}

/// Walk from `roots`. Returns the reached files and every declaration
/// that resolved to no file (which would not compile, so it means the walk is
/// reading the tree wrong).
fn reachable(roots: Vec<PathBuf>) -> (BTreeSet<PathBuf>, Vec<String>) {
	let mut reached = BTreeSet::new();
	let mut unresolved = Vec::new();
	let root_set: BTreeSet<PathBuf> = roots.iter().cloned().collect();
	let mut queue = roots;
	while let Some(file) = queue.pop() {
		if !reached.insert(file.clone()) {
			continue;
		}
		let src = fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
		for (name, path_attr) in mod_declarations(&src) {
			match declared_files(&file, root_set.contains(&file), &name, path_attr.as_deref())
				.into_iter()
				.find(|p| p.is_file())
			{
				Some(found) => queue.push(found),
				None => unresolved.push(format!("{}: mod {name}", file.display())),
			}
		}
	}
	(reached, unresolved)
}

#[test]
fn every_file_under_internal_is_reached_from_a_crate_root() {
	let internal = Path::new(env!("CARGO_MANIFEST_DIR")).join("internal");
	let mut sources = BTreeSet::new();
	all_sources(&internal, &mut sources);
	let (reached, unresolved) = reachable(vec![internal.join("lib.rs"), internal.join("main.rs")]);

	assert!(
		unresolved.is_empty(),
		"declarations the walk could not resolve to a file; the walk is reading \
		 the tree wrong, not the crate: {unresolved:#?}"
	);
	assert!(
		reached.len() > 200,
		"reached only {} files from lib.rs and main.rs; a walk that stops early \
		 reports every file as unreached",
		reached.len()
	);
	let orphans: Vec<_> = sources.difference(&reached).collect();
	assert!(
		orphans.is_empty(),
		"these files are never compiled, so nothing in them runs; declare them \
		 with `mod` (and `#[path]` for a sibling test file) or delete them: {orphans:#?}"
	);
}

/// The same for `tests/`. Cargo builds every `tests/*.rs` as its own crate,
/// so those are the roots; a file in a subdirectory runs only when one of
/// them declares it, which is how `tests/engine_integration/*.rs` are wired.
#[test]
fn every_file_under_tests_is_reached_from_a_test_crate() {
	let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
	let mut sources = BTreeSet::new();
	all_sources(&tests, &mut sources);
	let roots: Vec<PathBuf> = sources
		.iter()
		.filter(|p| p.parent() == Some(tests.as_path()))
		.cloned()
		.collect();
	assert!(
		roots.len() > 20,
		"found {} test crates; the scan of tests/ is broken",
		roots.len()
	);
	let (reached, unresolved) = reachable(roots);

	assert!(
		unresolved.is_empty(),
		"declarations the walk could not resolve to a file: {unresolved:#?}"
	);
	let orphans: Vec<_> = sources.difference(&reached).collect();
	assert!(
		orphans.is_empty(),
		"these test files are never compiled, so they never run: {orphans:#?}"
	);
}

/// The resolver is what the assertion above trusts, so pin the three shapes it
/// has to read: a plain child, a `#[path]` sibling behind another attribute,
/// and a child of a non-`mod.rs` file.
#[test]
fn the_walk_resolves_the_declaration_shapes_the_crate_uses() {
	// Built from pieces so no line of this file is itself a declaration.
	let src = [
		"use std::fmt;",
		"",
		"// comment",
		"#[cfg(test)]",
		"#[path = \"spec_body_tests.rs\"]",
		"mod spec_body_tests;",
		"pub(crate) mod fields;",
		"pub(in crate::engine) mod upload;",
		"#[path = \"not_a_mod.rs\"]",
		"fn stray() {}",
		"mod later;",
	]
	.join("\n");
	assert_eq!(
		mod_declarations(&src),
		vec![
			(
				"spec_body_tests".to_string(),
				Some("spec_body_tests.rs".to_string())
			),
			("fields".to_string(), None),
			("upload".to_string(), None),
			("later".to_string(), None),
		],
		"a #[path] must attach to the next mod, not survive a non-mod item"
	);

	let mod_rs = Path::new("/c/engine/container/mod.rs");
	assert_eq!(
		declared_files(mod_rs, false, "fields", None)[0],
		Path::new("/c/engine/container/fields.rs")
	);
	let plain = Path::new("/c/engine/copy.rs");
	assert_eq!(
		declared_files(plain, false, "stream", None)[0],
		Path::new("/c/engine/copy/stream.rs")
	);
	assert_eq!(
		declared_files(plain, false, "t", Some("copy/destination_tests.rs"))[0],
		Path::new("/c/engine/copy/destination_tests.rs")
	);
	// A test crate root declares `mod harness;` beside itself.
	assert_eq!(
		declared_files(
			Path::new("/c/tests/build_contract.rs"),
			true,
			"harness",
			None
		)[1],
		Path::new("/c/tests/harness/mod.rs")
	);
}
