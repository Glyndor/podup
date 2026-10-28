//! Filesystem walking helpers used by the engine to discover build contexts
//! and other recursively-resolved paths.
//!
//! Kept in its own module so `engine::mod` stays within the 500-line hard cap
//! that the org's `line-limit` reusable enforces. Pure and total so the
//! recursion is unit-testable without standing up a real build context.

use std::path::{Path, PathBuf};

mod opened;
pub(in crate::backends::engine) use opened::{visit_dir_skipping, Entry};

// Thread-local counter for `read_dir` calls inside the walk, used by the
// unit tests to assert that an ignored subtree is pruned before
// enumeration rather than walked and then filtered (#1746 entry 6).
// Compiled out of release builds.
#[cfg(test)]
std::thread_local! {
	static WALKED_PATHS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Reset the test-only walk counter. The test that pins the walk's
/// pruning behaviour calls this immediately before driving the
/// build flow, so the counter measures only the call under test and
/// not anything earlier in the process.
#[cfg(test)]
pub(in crate::backends::engine) fn reset_walk_counter() {
	WALKED_PATHS.with(|c| c.set(0));
}

/// Read the test-only walk counter. After `reset_walk_counter`, this
/// is the number of paths the walk pushed into its output. Pre-fix
/// every entry under an ignored subtree would be pushed (one per
/// file, plus the directory itself), so the counter for a context
/// with a 200-file `target/` would be ~201 higher than the same
/// context after the fix.
#[cfg(test)]
pub(in crate::backends::engine) fn walk_count() -> u64 {
	WALKED_PATHS.with(|c| c.get())
}

#[cfg(test)]
fn bump_walk_counter() {
	WALKED_PATHS.with(|c| c.set(c.get() + 1));
}

/// Recursive walk rooted at `root`, returning every entry (file or directory)
/// in deterministic path order. Pure helper; used by the build-context
/// materialisation paths to stage a tree before `podman build`.
pub(in crate::backends::engine) fn walk_dir(root: &Path) -> std::io::Result<Vec<PathBuf>> {
	let mut out = Vec::new();
	walk_collect(root, &mut out)?;
	Ok(out)
}

fn walk_collect(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
	let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
	entries.sort_by_key(|e| e.file_name());
	for entry in entries {
		let path = entry.path();
		let file_type = entry.file_type()?;
		out.push(path.clone());
		#[cfg(test)]
		bump_walk_counter();
		if file_type.is_dir() {
			walk_collect(&path, out)?;
		}
	}
	Ok(())
}

/// Collect the descriptor walk's paths for pruning and race regressions.
/// Production packing consumes entries directly through `visit_dir_skipping`
/// so it never reopens the collected names.
#[cfg(test)]
pub(in crate::backends::engine) fn walk_dir_skipping<F>(
	root: &Path,
	skip_dir: F,
) -> std::io::Result<Vec<PathBuf>>
where
	F: Fn(&Path) -> bool,
{
	let mut out = Vec::new();
	visit_dir_skipping(
		root,
		skip_dir,
		|_| true,
		|path, _entry| {
			out.push(path.to_path_buf());
			Ok(())
		},
	)?;
	Ok(out)
}

#[cfg(test)]
#[path = "walk_tests.rs"]
mod tests;
