//! The size and depth caps of the loader, each at its edge.
//!
//! Every case has a pair: a file just inside the cap, which must load, and one
//! just outside, which must be refused with the message of that cap. The
//! inside case is what keeps the outside one honest, since a loader that
//! refuses everything would pass the second alone.

use std::path::{Path, PathBuf};

use super::{assert_refused, assert_survived, compose_dir, config, write};

const MIB: usize = 1024 * 1024;

fn alias_document(anchor_bytes: usize, refs: usize) -> String {
	let anchor = "y".repeat(anchor_bytes);
	let uses = "      - *a\n".repeat(refs);
	format!("x-a: &a {anchor}\nservices:\n  web:\n    image: alpine\n    command:\n{uses}")
}

/// 90 references to a 60 000-byte anchor: under the reference count, the node
/// count and the document size, so only the expanded-content cap stops it.
#[test]
fn an_alias_expansion_past_its_byte_cap_is_refused() {
	let dir = compose_dir();
	let outcome = config(&write(dir.path(), "c.yaml", &alias_document(60_000, 90)));
	assert_refused("alias expansion over the cap", &outcome);
	assert!(
		outcome.text.contains("expanded alias content would reach"),
		"another guard refused it:\n{}",
		outcome.text
	);
}

#[test]
fn an_alias_expansion_under_its_byte_cap_loads() {
	let dir = compose_dir();
	let outcome = config(&write(dir.path(), "c.yaml", &alias_document(20_000, 90)));
	assert_survived("alias expansion under the cap", &outcome);
	assert_eq!(outcome.code, Some(0), "{}", outcome.text);
}

/// `f0` includes `f1`, which includes `f2`, down to `f{last}`.
fn include_chain(dir: &Path, last: usize) -> PathBuf {
	for index in 0..last {
		let next = index + 1;
		let body = format!("include: [f{next}.yaml]\n");
		write(dir, &format!("f{index}.yaml"), &body);
	}
	write(
		dir,
		&format!("f{last}.yaml"),
		"services:\n  web:\n    image: alpine\n",
	);
	dir.join("f0.yaml")
}

#[test]
fn an_include_chain_at_the_depth_cap_loads() {
	let dir = compose_dir();
	let outcome = config(&include_chain(dir.path(), 16));
	assert_survived("include chain of 16 levels", &outcome);
	assert_eq!(outcome.code, Some(0), "{}", outcome.text);
}

#[test]
fn an_include_chain_past_the_depth_cap_is_refused() {
	let dir = compose_dir();
	let outcome = config(&include_chain(dir.path(), 17));
	assert_refused("include chain of 17 levels", &outcome);
	assert!(
		outcome.text.contains("more than 16 levels deep"),
		"another guard refused it:\n{}",
		outcome.text
	);
}

/// One service per file, each extending a different file.
fn extends_fan(dir: &Path, files: usize) -> PathBuf {
	let mut compose = String::from("services:\n");
	for index in 0..files {
		write(
			dir,
			&format!("e{index}.yaml"),
			"services:\n  base:\n    image: alpine\n",
		);
		compose.push_str(&format!(
			"  s{index}:\n    extends: {{file: e{index}.yaml, service: base}}\n"
		));
	}
	write(dir, "c.yaml", &compose)
}

#[test]
fn extends_of_as_many_distinct_files_as_the_cache_holds_loads() {
	let dir = compose_dir();
	let outcome = config(&extends_fan(dir.path(), 256));
	assert_survived("256 distinct extends files", &outcome);
	assert_eq!(outcome.code, Some(0), "{}", outcome.text);
}

#[test]
fn extends_of_one_more_distinct_file_than_the_cache_holds_is_refused() {
	let dir = compose_dir();
	let outcome = config(&extends_fan(dir.path(), 257));
	assert_refused("257 distinct extends files", &outcome);
	assert!(
		outcome.text.contains("extends.file cache exceeded"),
		"another guard refused it:\n{}",
		outcome.text
	);
}

/// A valid compose file of exactly `size` bytes: the services, then a comment
/// padded so the whole file is `size` bytes long.
fn file_of_size(size: usize) -> String {
	let head = "services:\n  web:\n    image: alpine\n#";
	let padding = size - head.len() - 1;
	format!("{head}{}\n", "x".repeat(padding))
}

#[test]
fn a_compose_file_of_exactly_the_size_cap_loads() {
	let dir = compose_dir();
	let outcome = config(&write(dir.path(), "c.yaml", &file_of_size(16 * MIB)));
	assert_survived("a compose file of 16 MiB", &outcome);
	assert_eq!(outcome.code, Some(0), "{}", outcome.text);
}

#[test]
fn a_compose_file_one_byte_over_the_size_cap_is_refused() {
	let dir = compose_dir();
	let outcome = config(&write(dir.path(), "c.yaml", &file_of_size(16 * MIB + 1)));
	assert_refused("a compose file of 16 MiB and a byte", &outcome);
	assert!(
		outcome
			.text
			.contains("is larger than the 16777216 byte limit"),
		"another guard refused it:\n{}",
		outcome.text
	);
}

/// `-f -` reads the compose document from standard input under the same 16 MiB
/// cap as a file. An endless stream must be refused at the cap, in bounded
/// memory, instead of being read until the machine runs out.
#[test]
fn an_endless_compose_on_stdin_is_refused_at_the_cap() {
	let dir = compose_dir();
	let mut cmd = super::file_process::isolated(dir.path());
	cmd.args(["--socket", super::DEAD_SOCKET, "-f", "-", "config"]);
	let outcome = super::file_process::run_fed(
		cmd,
		Some(Box::new(|mut pipe| {
			use std::io::Write;
			let line =
				"x-filler: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n".repeat(1024);
			while pipe.write_all(line.as_bytes()).is_ok() {}
		})),
	);
	assert_refused("endless stdin", &outcome);
	assert!(
		outcome.text.contains("standard input is larger than"),
		"another guard refused it:\n{}",
		outcome.text
	);
}

#[test]
fn a_compose_on_stdin_under_the_cap_loads() {
	let dir = compose_dir();
	let mut cmd = super::file_process::isolated(dir.path());
	cmd.args(["--socket", super::DEAD_SOCKET, "-f", "-", "config"]);
	let outcome = super::file_process::run_fed(
		cmd,
		Some(Box::new(|mut pipe| {
			use std::io::Write;
			let _ = pipe.write_all(b"services:\n  web:\n    image: alpine\n");
		})),
	);
	assert_survived("small stdin", &outcome);
	assert_eq!(outcome.code, Some(0), "{}", outcome.text);
}
