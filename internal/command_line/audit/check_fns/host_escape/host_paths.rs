//! Path helpers shared by the checks that resolve a host path the compose file
//! names: where it lands once the engine resolves it, and its shortest
//! spelling.

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
