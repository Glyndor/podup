//! Path helpers shared by the checks that resolve a host path the compose file
//! names: where it lands once the engine resolves it, its shortest spelling,
//! and its canonical form.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// The host path `src` names, resolved the way the engine resolves a bind or
/// `file:` source: a leading `~` is the home directory, a relative path is
/// joined to the project directory. A path with a leading separator is kept
/// as written: it is absolute on every platform, while Windows would put the
/// drive of the project directory in front of it.
pub(super) fn resolve(src: &str, base_dir: &Path) -> PathBuf {
	if src.starts_with(['/', '\\']) {
		return PathBuf::from(src);
	}
	PathBuf::from(podup::resolve_bind_source(src, base_dir))
}

/// The host path the engine reads for a field it joins to the project
/// directory as written: `env_file:`, `build.context`, `build.dockerfile` and
/// a local `build.additional_contexts`. The engine does not expand `~` in
/// them, so `~` names a directory called `~` inside the project, and that is
/// the file it reads.
pub(super) fn join_as_written(src: &str, base_dir: &Path) -> PathBuf {
	base_dir.join(src)
}

/// `path` with every symbolic link resolved, or, when it does not exist, its
/// deepest ancestor that does, with the rest of the path appended. `None`
/// when the path cannot be resolved at all.
///
/// The ancestor decides where the path lands: a link in the middle of it can
/// move the whole tail somewhere else. The tail that does not exist yet keeps
/// the place it would take, so the result is the path the engine would read.
pub(super) fn canonical_form(path: &Path) -> Option<PathBuf> {
	let mut candidate = path.to_path_buf();
	let mut tail: Vec<OsString> = Vec::new();
	loop {
		if let Ok(real) = std::fs::canonicalize(&candidate) {
			let mut out = real;
			for part in tail.iter().rev() {
				out.push(part);
			}
			return Some(out);
		}
		tail.push(candidate.file_name()?.to_os_string());
		if !candidate.pop() {
			return None;
		}
	}
}

/// The home directory `~` expands to, read the way the engine reads it:
/// `HOME`, then `USERPROFILE`, an empty value counting as unset.
pub(super) fn home_dir() -> Option<PathBuf> {
	std::env::var_os("HOME")
		.or_else(|| std::env::var_os("USERPROFILE"))
		.filter(|v| !v.is_empty())
		.map(PathBuf::from)
}

/// Lexically fold `.` and `..` out of `path`, never climbing above the root.
/// The filesystem is not consulted, so a symbolic link is not followed.
pub(super) fn fold(path: &Path) -> PathBuf {
	let mut out = PathBuf::new();
	for component in path.components() {
		match component {
			Component::CurDir => {}
			Component::ParentDir => {
				if matches!(out.components().next_back(), Some(Component::Normal(_))) {
					out.pop();
				}
			}
			other => out.push(other.as_os_str()),
		}
	}
	out
}
