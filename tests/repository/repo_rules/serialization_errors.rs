//! A serialisation error must be propagated, not turned into a value.
//!
//! `serde_json::to_string(&value).unwrap_or_default()` answers "this value
//! cannot be serialised" with an empty string, and an empty string is
//! indistinguishable from a value that serialised to nothing. Where the
//! serialised text is the whole output of a command, the command prints an
//! empty document and exits zero, so a script reading it sees "no results"
//! where the truth is "the output could not be built" (#1444). Where the text
//! goes into a query parameter, libpod reads the empty string as "no value
//! given" and runs with the defaults the operator did not ask for (#1366).
//!
//! Both shapes are gone from this tree. The JSON helpers in
//! `internal/backends/engine/encoding/json.rs` map the error onto the command's
//! error type, and the statistics frame propagates it the same way. Nothing
//! reads like the old form any more, so the rule can be a check instead of a
//! memory.
//!
//! Clippy cannot say it: `disallowed_methods` names a method, and the empty
//! string comes from what is chained onto the call, not from the call itself.
//!
//! This is the check. It reads the sources of the package and fails when a
//! serialisation result is turned into an empty or a default value. The
//! reading is at the token level, so the shape is found wherever rustfmt put
//! it: a chain split over three lines is the chain the rule is about, and
//! reformatting the file does not make the defect go away.
//!
//! The shapes it must not report are pinned in `cases.rs`, with the reason
//! each one is allowed, so a rule that quietly grew wider is visible.

#[path = "serialization_errors/cases.rs"]
mod cases;
#[path = "serialization_errors/detector.rs"]
mod detector;
#[path = "serialization_errors/lexer.rs"]
mod lexer;

use std::fs;
use std::path::{Path, PathBuf};

/// The sources the rule covers.
///
/// The crate that ships, the suite that guards it, and the build script that
/// runs before either of them. The benchmark workspace and the fuzz targets
/// are separate crates with their own manifests, and they read no compose
/// model, so they are not part of what this rule is about.
fn sources(root: &Path) -> Vec<PathBuf> {
	let mut files = Vec::new();
	collect(&root.join("internal"), &mut files);
	collect(&root.join("tests"), &mut files);
	let build = root.join("build.rs");
	if build.is_file() {
		files.push(build);
	}
	files
}

/// Every `.rs` file under `dir`.
fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
	let Ok(entries) = fs::read_dir(dir) else {
		return;
	};
	for entry in entries.flatten() {
		let path = entry.path();
		if path.is_dir() {
			collect(&path, files);
		} else if path.extension().is_some_and(|e| e == "rs") {
			files.push(path);
		}
	}
}

#[test]
fn no_serialisation_error_is_turned_into_an_empty_value() {
	let root = Path::new(env!("CARGO_MANIFEST_DIR"));
	let files = sources(root);
	assert!(!files.is_empty(), "found source files to scan");
	let mut offenders = Vec::new();
	for file in &files {
		let source = fs::read_to_string(file).expect("read source");
		let tokens = lexer::tokenize(&source);
		for finding in detector::findings(&tokens) {
			offenders.push(format!(
				"{}:{}: a serialisation error becomes a value through `{}`",
				file.display(),
				finding.line,
				finding.shape
			));
		}
	}
	assert!(
		offenders.is_empty(),
		"a serialisation error must be propagated, not turned into a value:\n{}",
		offenders.join("\n")
	);
}
