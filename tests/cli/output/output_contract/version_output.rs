//! `version`, `--version` and `version --short` answer the same question in
//! one shape each.
//!
//! Split out from `tests/cli/output/output_contract.rs`; the shared fixtures are declared
//! in the crate root, which the scenario files inherit via `use super::*`.

use std::process::Command;

use super::*;
use harness::bin;

/// `podup version` and `podup --version` answer the same question, so they emit
/// the same line.
///
/// They did not: clap's derived `--version` rendered `podup 3.3.0` while the
/// subcommand rendered `podup version v3.3.0`, so a script probing the binary
/// got a different string depending on which spelling it happened to use, and
/// the `v` prefix, which is what the tags and the release assets carry, appeared
/// in only one of them. Measured on docker-compose v5.1.3, both spellings return
/// `Docker Compose version v5.1.3` byte for byte.
#[test]
fn version_subcommand_and_flag_agree() {
	let sub = Command::new(bin()).arg("version").output().unwrap();
	let flag = Command::new(bin()).arg("--version").output().unwrap();
	let sub = String::from_utf8_lossy(&sub.stdout).trim().to_string();
	let flag = String::from_utf8_lossy(&flag.stdout).trim().to_string();
	assert_eq!(sub, flag, "`version` and `--version` must emit one line");
	assert!(
		sub.starts_with("podup version v"),
		"expected `podup version v<semver>`, got {sub:?}"
	);
}

/// `version --short` is the one spelling that drops the `v`, so a script can get
/// a bare semver without parsing the sentence. The reference behaves the same
/// way (`docker-compose version --short` returns `5.1.3`).
#[test]
fn version_short_is_a_bare_semver() {
	let out = Command::new(bin())
		.args(["version", "--short"])
		.output()
		.unwrap();
	let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
	assert!(
		!line.starts_with('v') && line.starts_with(|c: char| c.is_ascii_digit()),
		"expected a bare semver with no `v`, got {line:?}"
	);
}
