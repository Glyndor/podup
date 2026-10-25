//! Timers an earlier run wrote that this run does not.
//!
//! Removing a `schedule` entry and generating again leaves the old `.timer` in
//! the output directory, and if it was installed it keeps firing. Nothing is
//! deleted here; the files are only named so the operator can remove them.
//!
//! The directory is whatever the operator pointed `-o` at, so a name ending in
//! `.timer` may be a FIFO, a device or a symlink. Only a regular file that is
//! not reached through a symlink is read, and the read itself is capped.

use std::io::Read;
use std::path::Path;

use crate::quadlet::QuadletUnit;

/// Largest timer file read when looking for the ownership marker.
const MAX_TIMER_BYTES: u64 = 16 * 1024;

/// File names in `dir` that are `.timer` units owned by `project` and are not
/// among `units`, sorted. An unreadable or missing directory has none.
pub fn stale_timers(dir: &Path, project: &str, units: &[QuadletUnit]) -> Vec<String> {
	let Ok(entries) = std::fs::read_dir(dir) else {
		return Vec::new();
	};
	let marker = format!("# podup-owner: {project}");
	let mut stale: Vec<String> = entries
		.filter_map(Result::ok)
		.filter_map(|entry| {
			let name = entry.file_name().into_string().ok()?;
			if !name.ends_with(".timer") || units.iter().any(|unit| unit.filename == name) {
				return None;
			}
			let text = read_small_regular_file(&entry.path())?;
			text.lines().any(|line| line == marker).then_some(name)
		})
		.collect();
	stale.sort();
	stale
}

/// The text of a regular file of at most [`MAX_TIMER_BYTES`], or `None` for
/// anything else: a symlink (not followed), a FIFO or device, a larger file, or
/// text that is not UTF-8. The type is checked on the opened descriptor and the
/// read is capped, so a file swapped in after a check cannot be read past the cap.
fn read_small_regular_file(path: &Path) -> Option<String> {
	// A cheap first look that does not follow links or open anything.
	if !std::fs::symlink_metadata(path).ok()?.is_file() {
		return None;
	}
	let file = open_without_following(path).ok()?;
	if !file.metadata().ok()?.is_file() {
		return None;
	}
	let mut bytes = Vec::new();
	file.take(MAX_TIMER_BYTES + 1)
		.read_to_end(&mut bytes)
		.ok()?;
	if bytes.len() as u64 > MAX_TIMER_BYTES {
		return None;
	}
	String::from_utf8(bytes).ok()
}

#[cfg(unix)]
fn open_without_following(path: &Path) -> std::io::Result<std::fs::File> {
	use std::os::unix::fs::OpenOptionsExt;
	// O_NOFOLLOW refuses a symlink, O_NONBLOCK keeps the open of a FIFO from
	// waiting for a writer.
	std::fs::OpenOptions::new()
		.read(true)
		.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
		.open(path)
}

#[cfg(not(unix))]
fn open_without_following(path: &Path) -> std::io::Result<std::fs::File> {
	std::fs::File::open(path)
}

#[cfg(test)]
#[path = "stale_tests.rs"]
mod tests;
