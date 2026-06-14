//! Filesystem helpers shared across parsing paths.

use std::io;
use std::io::Read;
use std::path::{Component, Path};

/// Reject a file reference that is absolute or escapes the project directory.
///
/// podup only reads files that live at or below the directory of the compose
/// file being processed. A reference that is absolute, rooted (`/etc/passwd`,
/// which is not `is_absolute()` on Windows but still escapes via the root
/// separator), or contains a `..` component is refused so a compose file cannot
/// turn podup into a confused deputy that reads arbitrary host paths. Shared by
/// the `extends`, `env_file`, and `label_file` readers.
pub(crate) fn is_safe_relative_path(path: impl AsRef<Path>) -> bool {
	let fp = path.as_ref();
	if fp.is_absolute() {
		return false;
	}
	let mut comps = fp.components();
	if comps.clone().next() == Some(Component::RootDir) {
		return false;
	}
	!comps.any(|c| c == Component::ParentDir)
}

/// Upper bound on the size of any compose, include, extends, or env file podup
/// will read into memory. Bounds memory use on an accidentally huge or hostile
/// input before it reaches the substitution and YAML stages.
pub(crate) const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Read a file to a `String`, refusing inputs larger than [`MAX_FILE_BYTES`].
///
/// A drop-in replacement for [`std::fs::read_to_string`] that fails closed with
/// an `InvalidData` error instead of allocating an unbounded buffer.
pub(crate) fn read_to_string_capped(path: impl AsRef<Path>) -> io::Result<String> {
	read_to_string_capped_with(path.as_ref(), MAX_FILE_BYTES)
}

fn read_to_string_capped_with(path: &Path, max: u64) -> io::Result<String> {
	// Read through a single file handle capped at `max + 1` bytes. Reading
	// rather than stat-then-read closes the TOCTOU window: a writer that grows
	// the file (or swaps in a symlink) after a size check cannot make podup
	// read past the cap, because the limit is enforced on the read itself.
	let file = std::fs::File::open(path)?;
	let mut buf = String::new();
	let read = file.take(max + 1).read_to_string(&mut buf)?;
	if read as u64 > max {
		return Err(io::Error::new(
			io::ErrorKind::InvalidData,
			format!("{} is larger than the {max} byte limit", path.display()),
		));
	}
	Ok(buf)
}

#[cfg(test)]
mod tests {
	use super::{is_safe_relative_path, read_to_string_capped_with};

	#[test]
	fn accepts_relative_subpaths() {
		assert!(is_safe_relative_path("labels.env"));
		assert!(is_safe_relative_path("config/labels.env"));
	}

	#[test]
	fn rejects_absolute_paths() {
		assert!(!is_safe_relative_path("/etc/passwd"));
	}

	#[test]
	fn rejects_parent_traversal() {
		assert!(!is_safe_relative_path("../secret.env"));
		assert!(!is_safe_relative_path("a/../../etc/passwd"));
	}

	#[test]
	fn reads_file_within_limit() {
		let dir = tempfile::tempdir().expect("tempdir");
		let f = dir.path().join("ok");
		std::fs::write(&f, b"hello").expect("write");
		assert_eq!(read_to_string_capped_with(&f, 16).unwrap(), "hello");
	}

	#[test]
	fn rejects_file_over_limit() {
		let dir = tempfile::tempdir().expect("tempdir");
		let f = dir.path().join("big");
		std::fs::write(&f, vec![b'x'; 32]).expect("write");
		let err = read_to_string_capped_with(&f, 16).unwrap_err();
		assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
	}

	#[test]
	fn missing_file_is_error() {
		assert!(read_to_string_capped_with(std::path::Path::new("/no/such/file"), 16).is_err());
	}
}
