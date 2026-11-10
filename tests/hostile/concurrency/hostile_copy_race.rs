//! `cp SERVICE:PATH DEST` while the destination is swapped for a symlink.
//!
//! podup asks the engine for the archive, looks at the destination, then
//! writes. Anyone who can write next to the destination can replace it with a
//! link to a directory elsewhere in the window between those steps. The
//! fake engine does the swap itself, at the moment the archive is asked for, so
//! the order is exact and no timing is involved. Whatever podup does next, the
//! target of the link must stay as it was. The layers that refuse it (the routing
//! check, the destination guard and the `O_NOFOLLOW` open of the extractor) each
//! stop the write alone, so the case fails only when all three are removed.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::sync::Mutex;

#[path = "../support/fake_engine.rs"]
mod fake_engine;
#[path = "../support/host_files.rs"]
mod host_files;

use fake_engine::{FakeEngine, Reply};
use host_files::{Running, MARKER, SAFE};

/// A one-file ustar archive named `f.txt` holding the in-tree control text.
fn archive() -> String {
	let mut header = [0u8; 512];
	header[..5].copy_from_slice(b"f.txt");
	header[100..108].copy_from_slice(b"0000644\0");
	header[108..116].copy_from_slice(b"0000000\0");
	header[116..124].copy_from_slice(b"0000000\0");
	header[124..136].copy_from_slice(format!("{:011o}\0", SAFE.len()).as_bytes());
	header[136..148].copy_from_slice(b"00000000000\0");
	header[148..156].copy_from_slice(b"        ");
	header[156] = b'0';
	header[257..265].copy_from_slice(b"ustar\x0000");
	let sum: u32 = header.iter().map(|b| u32::from(*b)).sum();
	header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
	let mut out = header.to_vec();
	out.extend_from_slice(SAFE.as_bytes());
	out.resize(out.len().div_ceil(512) * 512 + 1024, 0);
	String::from_utf8(out).expect("the archive is ASCII and NUL")
}

/// What to do to the destination when the archive is asked for: the path to
/// replace and what to point the replacement at.
static SWAP: Mutex<Option<(PathBuf, PathBuf)>> = Mutex::new(None);

/// The swap above is process-wide, so the cases take turns.
static TURN: Mutex<()> = Mutex::new(());

fn swapping(line: &str, _: &str) -> Option<Reply> {
	if !line.starts_with("GET") || !line.contains("/archive") {
		return None;
	}
	if let Some((path, target)) = SWAP.lock().unwrap().take() {
		if path.is_dir() {
			fs::remove_dir_all(&path).unwrap();
		} else if path.exists() {
			fs::remove_file(&path).unwrap();
		}
		symlink(target, path).unwrap();
	}
	Some(Reply {
		status: 200,
		body: archive(),
	})
}

fn engine() -> FakeEngine {
	let fake = FakeEngine::start(swapping);
	fs::write(
		fake.dir().join("compose.yaml"),
		"services:\n  web:\n    image: alpine\n",
	)
	.unwrap();
	fs::create_dir(fake.dir().join("work")).unwrap();
	fs::create_dir(fake.dir().join("outside")).unwrap();
	fs::write(fake.dir().join("outside/victim"), MARKER).unwrap();
	fake
}

/// Run `cp web:/data/f.txt <dest>` and return the exit status and output.
fn cp(fake: &FakeEngine, dest: &std::path::Path) -> (bool, String) {
	let mut child = Running::start(fake, &["cp", "web:/data/f.txt", dest.to_str().unwrap()]);
	let ok = child.wait().success();
	(ok, child.text())
}

fn outside_untouched(fake: &FakeEngine) {
	let outside = fake.dir().join("outside");
	assert_eq!(fs::read(outside.join("victim")).unwrap(), MARKER.as_bytes());
	let names: Vec<_> = fs::read_dir(&outside)
		.unwrap()
		.map(|e| e.unwrap().file_name())
		.collect();
	assert_eq!(names.len(), 1, "something was created outside: {names:?}");
}

#[test]
fn a_directory_destination_swapped_for_a_link_is_not_written_through() {
	let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
	let fake = engine();
	let dest = fake.dir().join("work");
	*SWAP.lock().unwrap() = Some((dest.clone(), fake.dir().join("outside")));
	let (_, text) = cp(&fake, &dest);
	assert!(
		SWAP.lock().unwrap().is_none(),
		"the swap did not run: {text}"
	);
	outside_untouched(&fake);
}

#[test]
fn the_control_without_a_swap_lands_in_the_destination() {
	let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
	// If cp never wrote anything, the two cases above would pass for the wrong
	// reason.
	let fake = engine();
	let dest = fake.dir().join("work/copy.txt");
	let (ok, text) = cp(&fake, &dest);
	assert!(ok, "{text}");
	assert_eq!(fs::read(dest).unwrap(), SAFE.as_bytes());
}
