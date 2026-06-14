//! Filesystem helpers shared across parsing paths.

use std::io;
use std::path::Path;

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
	let meta = std::fs::metadata(path)?;
	if meta.len() > max {
		return Err(io::Error::new(
			io::ErrorKind::InvalidData,
			format!(
				"{} is {} bytes, larger than the {max} byte limit",
				path.display(),
				meta.len()
			),
		));
	}
	std::fs::read_to_string(path)
}

#[cfg(test)]
mod tests {
	use super::read_to_string_capped_with;

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
