//! Preserve holes while reading tar's expanded sparse-file stream.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};

/// Copy sparse contents with bounded memory and without allocating zero runs on disk.
pub(super) fn copy<R: Read>(entry: &mut tar::Entry<'_, R>, output: &mut File) -> io::Result<()> {
	let expected = entry.size();
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
