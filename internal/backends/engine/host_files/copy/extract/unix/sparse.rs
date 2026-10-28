//! Preserve holes while reading tar's expanded sparse-file stream.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};

/// The largest logical size a sparse entry may declare. The size comes from the
/// archive, and the zero runs cost no bytes in it, so a few hundred bytes of
/// header can announce any length; reading and seeking over that many zeros
/// is time spent for nothing. 64 GiB is far above any file `cp` is meant to
/// bring out of a container and still finishes in seconds.
const MAX_SPARSE_LOGICAL_BYTES: u64 = 64 * 1024 * 1024 * 1024;

/// Copy sparse contents with bounded memory and without allocating zero runs on disk.
pub(super) fn copy<R: Read>(entry: &mut tar::Entry<'_, R>, output: &mut File) -> io::Result<()> {
	let expected = entry.size();
	if expected > MAX_SPARSE_LOGICAL_BYTES {
		return Err(io::Error::new(
			io::ErrorKind::InvalidData,
			format!("cp: sparse entry declares {expected} bytes, over the {MAX_SPARSE_LOGICAL_BYTES} byte limit"),
		));
	}
	let mut copied = 0_u64;
	let mut buffer = [0_u8; 64 * 1024];
	loop {
		let n = entry.read(&mut buffer)?;
		if n == 0 {
			break;
		}
		// Entry::Read synthesizes the sparse padding. Seeking instead of
		// writing it preserves holes, including a trailing zero run.
		if buffer[..n].iter().all(|&byte| byte == 0) {
			output.seek(SeekFrom::Current(n as i64))?;
		} else {
			output.write_all(&buffer[..n])?;
		}
		copied += n as u64;
	}
	if copied != expected {
		return Err(io::Error::new(
			io::ErrorKind::UnexpectedEof,
			"cp: incomplete sparse entry",
		));
	}
	output.set_len(expected)
}
