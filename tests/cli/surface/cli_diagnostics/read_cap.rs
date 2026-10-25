//! The 16 MiB read cap, over `-f -` and over a file on disk: past the cap is
//! refused, under the cap is accepted.
//!
//! Split out from `tests/cli/surface/cli_diagnostics.rs`; the helpers (`bin`, `TempDir`)
//! are inherited via `use super::*`.

use super::*;

/// The `-f -` form enforces the same 16 MiB read cap the file path does.
///
/// The cap exists so a pathological compose document cannot exhaust memory, and
/// the generic reader that implements it is unit-tested, but with an explicit
/// limit passed in. **Nothing checked that the stdin call site passes
/// `MAX_FILE_BYTES` rather than something larger**, which a mutation replacing it
/// with `u64::MAX` proved by surviving the whole suite.
///
/// It cannot be closed at unit level: the function reads the real stdin, so the
/// only honest test is the one a user would perform. This feeds the binary more
/// than the cap through a pipe and asserts it is refused.
#[test]
fn stdin_is_refused_past_the_read_cap() {
	use std::io::Write;
	use std::process::Stdio;

	// One byte over 16 MiB, and valid YAML up to the point it is rejected, so a
	// refusal cannot be mistaken for a parse error. The padding lives in a
	// comment for the same reason.
	const CAP: usize = 16 * 1024 * 1024;
	let mut document = String::from("services:\n  web:\n    image: alpine\n# ");
	document.push_str(&"x".repeat(CAP + 1 - document.len()));
	document.push('\n');
	assert!(document.len() > CAP, "the fixture must exceed the cap");

	let mut child = Command::new(bin())
		.args(["-f", "-", "config"])
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	// A refused read closes the pipe, so the write can fail with EPIPE before
	// the whole document is sent. That is the success path, not an error.
	let _ = child.stdin.take().unwrap().write_all(document.as_bytes());
	let out = child.wait_with_output().unwrap();

	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		!out.status.success(),
		"oversized stdin was accepted: stdout {} bytes, stderr {stderr:?}",
		out.stdout.len()
	);
	assert!(
		stderr.contains("larger than") && stderr.contains("limit"),
		"refused for some other reason than the cap: {stderr:?}"
	);
}

/// The same document one byte under the cap is accepted, so the test above
/// pins a threshold rather than "big inputs fail".
#[test]
fn stdin_just_under_the_cap_is_accepted() {
	use std::io::Write;
	use std::process::Stdio;

	const CAP: usize = 16 * 1024 * 1024;
	let mut document = String::from("services:\n  web:\n    image: alpine\n# ");
	document.push_str(&"x".repeat(CAP - document.len() - 1));
	document.push('\n');
	assert!(document.len() <= CAP, "the fixture must fit under the cap");

	let mut child = Command::new(bin())
		.args(["-f", "-", "config"])
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	child
		.stdin
		.take()
		.unwrap()
		.write_all(document.as_bytes())
		.unwrap();
	let out = child.wait_with_output().unwrap();
	assert!(
		out.status.success(),
		"a document under the cap was refused: {}",
		String::from_utf8_lossy(&out.stderr)
	);
}

/// A compose file past the 16 MiB cap is refused, the same as stdin.
///
/// The cap is a bound on what a single trusted-but-unbounded input can make
/// podup allocate. `read_capped_from` implements it and is unit-tested, with an
/// explicit limit passed in, so **what was untested is that the real call sites
/// pass `MAX_FILE_BYTES`**. A mutation raising the constant to a terabyte left
/// the whole lib and bins suite green, which is what a control nothing exercises
/// looks like from outside.
#[test]
fn a_compose_file_past_the_read_cap_is_refused() {
	const CAP: usize = 16 * 1024 * 1024;
	let dir = TempDir::new().unwrap();
	let path = dir.path().join("docker-compose.yml");
	let mut document = String::from("services:\n  web:\n    image: alpine\n# ");
	document.push_str(&"x".repeat(CAP + 1 - document.len()));
	document.push('\n');
	fs::write(&path, &document).unwrap();

	let out = Command::new(bin())
		.args(["-f", path.to_str().unwrap(), "config"])
		.output()
		.unwrap();
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		!out.status.success(),
		"an oversized compose file was accepted: {} bytes of stdout",
		out.stdout.len()
	);
	assert!(
		stderr.contains("larger than") && stderr.contains("limit"),
		"refused for some other reason than the cap: {stderr:?}"
	);
}

/// The same file one byte under the cap is accepted, so the test above pins the
/// threshold rather than "large files fail".
#[test]
fn a_compose_file_just_under_the_read_cap_is_accepted() {
	const CAP: usize = 16 * 1024 * 1024;
	let dir = TempDir::new().unwrap();
	let path = dir.path().join("docker-compose.yml");
	let mut document = String::from("services:\n  web:\n    image: alpine\n# ");
	document.push_str(&"x".repeat(CAP - document.len() - 1));
	document.push('\n');
	fs::write(&path, &document).unwrap();

	let out = Command::new(bin())
		.args(["-f", path.to_str().unwrap(), "config"])
		.output()
		.unwrap();
	assert!(
		out.status.success(),
		"a compose file under the cap was refused: {}",
		String::from_utf8_lossy(&out.stderr)
	);
}
