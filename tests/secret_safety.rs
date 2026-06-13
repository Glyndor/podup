//! Regression guard for the secret-safety rule: diagnostics and logs must print
//! secret/config/env **names**, never their resolved **values**.
//!
//! The engine already complies (it logs `{name}`, stages bytes without logging
//! them, and errors reference `/run/secrets/{name}` rather than contents). This
//! test fails closed if a future change adds a logging or printing macro that
//! interpolates a binding whose name marks it as a secret value, so a stray
//! `debug!("{value}")` on the env/secret path cannot regress unnoticed.

use std::fs;
use std::path::{Path, PathBuf};

/// Output macros that reach a user-visible channel (logs, stdout, stderr).
const OUTPUT_MACROS: &[&str] = &[
	"trace!",
	"debug!",
	"info!",
	"warn!",
	"error!",
	"eprintln!",
	"println!",
	"eprint!",
	"print!",
];

/// Interpolation fragments that name a resolved secret/env *value* rather than
/// a key. A logging line containing one of these is almost certainly a leak.
const BANNED_INTERPOLATIONS: &[&str] = &[
	"{value}",
	"{value:",
	"{val}",
	"{secret}",
	"{password}",
	"{passwd}",
	"{token}",
	"{contents}",
	"{plaintext}",
];

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
	for entry in fs::read_dir(dir).expect("read internal/ dir") {
		let path = entry.expect("dir entry").path();
		if path.is_dir() {
			rust_sources(&path, out);
		} else if path.extension().is_some_and(|e| e == "rs") {
			out.push(path);
		}
	}
}

#[test]
fn output_macros_never_interpolate_secret_values() {
	let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("internal");
	let mut files = Vec::new();
	rust_sources(&root, &mut files);
	assert!(!files.is_empty(), "found source files to scan");

	let mut offenders = Vec::new();
	for file in &files {
		let text = fs::read_to_string(file).expect("read source");
		for (lineno, line) in text.lines().enumerate() {
			let is_output = OUTPUT_MACROS.iter().any(|m| line.contains(m));
			if !is_output {
				continue;
			}
			if let Some(bad) = BANNED_INTERPOLATIONS.iter().find(|b| line.contains(**b)) {
				offenders.push(format!(
					"{}:{}: interpolates {bad} into output",
					file.display(),
					lineno + 1
				));
			}
		}
	}
	assert!(
		offenders.is_empty(),
		"output must log secret/env names, not values:\n{}",
		offenders.join("\n")
	);
}
