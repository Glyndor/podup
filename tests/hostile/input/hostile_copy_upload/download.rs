//! `cp SERVICE:PATH DEST` against a Podman that answers with chosen archives.
//!
//! A destination that is an existing directory streams the archive into the
//! extractor; any other destination buffers it first. Each route gets the same
//! two archives: a plain file, which has to land where it was named, and one
//! whose entry climbs out of the destination, which must leave nothing outside.

use std::fs;

use super::fake_engine::{FakeEngine, Reply};
use super::host_files::{Running, MARKER, SAFE};

/// A ustar archive of regular files, `(name, content)` each.
fn tar(entries: &[(&str, &str)]) -> String {
	let mut out = Vec::new();
	for (name, content) in entries {
		let mut header = [0u8; 512];
		header[..name.len()].copy_from_slice(name.as_bytes());
		header[100..108].copy_from_slice(b"0000644\0");
		header[108..116].copy_from_slice(b"0000000\0");
		header[116..124].copy_from_slice(b"0000000\0");
		header[124..136].copy_from_slice(format!("{:011o}\0", content.len()).as_bytes());
		header[136..148].copy_from_slice(b"00000000000\0");
		header[148..156].copy_from_slice(b"        ");
		header[156] = b'0';
		header[257..265].copy_from_slice(b"ustar\x0000");
		let sum: u32 = header.iter().map(|b| u32::from(*b)).sum();
		header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
		out.extend_from_slice(&header);
		out.extend_from_slice(content.as_bytes());
		out.resize(out.len().div_ceil(512) * 512, 0);
	}
	out.resize(out.len() + 1024, 0);
	String::from_utf8(out).expect("the archive is ASCII and NUL")
}

fn plain(line: &str, _: &str) -> Option<Reply> {
	line.contains("/archive").then(|| Reply {
		status: 200,
		body: tar(&[("f.txt", SAFE)]),
	})
}

fn climbing(line: &str, _: &str) -> Option<Reply> {
	line.contains("/archive").then(|| Reply {
		status: 200,
		body: tar(&[("../pwn.txt", MARKER), ("f.txt", SAFE)]),
	})
}

/// One climbing entry alone: the buffered route reads it as a single file.
fn climbing_alone(line: &str, _: &str) -> Option<Reply> {
	line.contains("/archive").then(|| Reply {
		status: 200,
		body: tar(&[("../pwn.txt", MARKER)]),
	})
}

fn engine(handler: fn(&str, &str) -> Option<Reply>) -> FakeEngine {
	let fake = FakeEngine::start(handler);
	fs::write(
		fake.dir().join("compose.yaml"),
		"services:\n  web:\n    image: alpine\n",
	)
	.unwrap();
	fs::create_dir(fake.dir().join("work")).unwrap();
	fake
}

/// Run `cp web:/data/f.txt <dest>` from `work/` and report the exit status.
fn cp(fake: &FakeEngine, dest: &str) -> (bool, String) {
	let mut child = Running::start(fake, &["cp", "web:/data/f.txt", dest]);
	let status = child.wait();
	(status.success(), child.text())
}

fn nothing_outside(fake: &FakeEngine) {
	for name in ["pwn.txt", "work/pwn.txt"] {
		let path = fake.dir().join(name);
		assert!(!path.exists(), "{} was written", path.display());
	}
}

#[test]
fn a_file_copied_to_a_new_path_lands_there() {
	let fake = engine(plain);
	let dest = fake.dir().join("copy.txt");
	let (ok, text) = cp(&fake, dest.to_str().unwrap());
	assert!(ok, "{text}");
	assert_eq!(fs::read(dest).unwrap(), SAFE.as_bytes());
}

#[test]
fn a_file_copied_into_a_directory_lands_inside_it() {
	let fake = engine(plain);
	let dest = fake.dir().join("work");
	let (ok, text) = cp(&fake, dest.to_str().unwrap());
	assert!(ok, "{text}");
	assert_eq!(fs::read(dest.join("f.txt")).unwrap(), SAFE.as_bytes());
}

#[test]
fn a_climbing_name_alone_is_written_only_at_the_named_destination() {
	let fake = engine(climbing_alone);
	let dest = fake.dir().join("work/new");
	let (ok, text) = cp(&fake, dest.to_str().unwrap());
	nothing_outside(&fake);
	// A single entry lands at the name the user gave, whatever the archive
	// called it, so the bytes are at `new` and never at `../pwn.txt`.
	assert!(ok, "{text}");
	assert_eq!(fs::read(dest).unwrap(), MARKER.as_bytes());
}

#[test]
fn a_climbing_entry_is_not_written_through_the_streaming_route() {
	let fake = engine(climbing);
	let dest = fake.dir().join("work");
	let (ok, text) = cp(&fake, dest.to_str().unwrap());
	nothing_outside(&fake);
	assert!(!ok && text.contains("escapes destination"), "{text}");
}
