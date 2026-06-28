//! Filesystem helpers shared across parsing paths.

use std::io;
use std::io::Read;
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
	// Read through a single file handle capped at `max + 1` bytes. Reading
	// rather than stat-then-read closes the TOCTOU window: a writer that grows
	// the file (or swaps in a symlink) after a size check cannot make podup
	// read past the cap, because the limit is enforced on the read itself.
	let file = std::fs::File::open(path)?;
	read_capped_from(file, max, &path.display().to_string())
}

/// Read the compose document from standard input, refusing input larger than
/// [`MAX_FILE_BYTES`]. Backs the `-f -` form (`cat compose.yaml | podup config
/// -f -`), which `docker compose` supports by reading the file from stdin.
pub(crate) fn read_stdin_to_string_capped() -> io::Result<String> {
	read_capped_from(io::stdin().lock(), MAX_FILE_BYTES, "standard input")
}

/// Read any [`io::Read`] into a `String`, enforcing the `max`-byte cap on the
/// read itself. `label` names the source for the over-limit error message.
fn read_capped_from(reader: impl Read, max: u64, label: &str) -> io::Result<String> {
	let mut buf = String::new();
	let read = reader.take(max + 1).read_to_string(&mut buf)?;
	if read as u64 > max {
		return Err(io::Error::new(
			io::ErrorKind::InvalidData,
			format!("{label} is larger than the {max} byte limit"),
		));
	}
	Ok(buf)
}

/// Read a file to a `Vec<u8>`, refusing inputs larger than [`MAX_FILE_BYTES`].
///
/// The bytes counterpart of [`read_to_string_capped`] for inputs that are not
/// necessarily UTF-8 (e.g. binary build-secret material). Fails closed with an
/// `InvalidData` error instead of allocating an unbounded buffer.
pub(crate) fn read_capped(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
	read_capped_with(path.as_ref(), MAX_FILE_BYTES)
}

fn read_capped_with(path: &Path, max: u64) -> io::Result<Vec<u8>> {
	// Same single-handle, cap-on-the-read strategy as the string variant: the
	// limit is enforced on the read itself, so a file that grows (or a symlink
	// swapped in) after any size check cannot push podup past the cap.
	let file = std::fs::File::open(path)?;
	let mut buf = Vec::new();
	let read = file.take(max + 1).read_to_end(&mut buf)?;
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
	use super::{read_capped_from, read_capped_with, read_to_string_capped_with};

	#[test]
	fn read_capped_from_reads_within_limit() {
		// The shared reader (used for both files and stdin) returns the content
		// untouched when it fits under the cap.
		let out = read_capped_from(std::io::Cursor::new(b"version: 1"), 64, "stdin").unwrap();
		assert_eq!(out, "version: 1");
	}

	#[test]
	fn read_capped_from_rejects_over_limit() {
		let err = read_capped_from(std::io::Cursor::new(vec![b'x'; 32]), 16, "stdin").unwrap_err();
		assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
		assert!(err.to_string().contains("stdin"));
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

	#[test]
	fn read_capped_reads_bytes_within_limit() {
		let dir = tempfile::tempdir().expect("tempdir");
		let f = dir.path().join("ok");
		// Non-UTF-8 bytes must round-trip: the bytes reader does no UTF-8 check.
		std::fs::write(&f, [0xff, 0x00, 0xfe]).expect("write");
		assert_eq!(read_capped_with(&f, 16).unwrap(), vec![0xff, 0x00, 0xfe]);
	}

	#[test]
	fn read_capped_rejects_bytes_over_limit() {
		let dir = tempfile::tempdir().expect("tempdir");
		let f = dir.path().join("big");
		std::fs::write(&f, vec![b'x'; 32]).expect("write");
		let err = read_capped_with(&f, 16).unwrap_err();
		assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
	}

	#[test]
	fn read_capped_missing_file_is_error() {
		assert!(read_capped_with(std::path::Path::new("/no/such/file"), 16).is_err());
	}
}
