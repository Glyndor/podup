//! Timers an earlier run wrote that this run does not.
//!
//! Removing a `schedule` entry and generating again leaves the old `.timer` in
//! the output directory, and if it was installed it keeps firing. Nothing is
//! deleted here; the files are only named so the operator can remove them.

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
			let path = entry.path();
			if std::fs::metadata(&path).ok()?.len() > MAX_TIMER_BYTES {
				return None;
			}
			let text = std::fs::read_to_string(&path).ok()?;
			text.lines().any(|line| line == marker).then_some(name)
		})
		.collect();
	stale.sort();
	stale
}

#[cfg(test)]
#[path = "stale_tests.rs"]
mod tests;
