//! Reading and replacing a unit file that may already exist in the output
//! directory, without trusting what is sitting at that name.

use std::io;
use std::path::{Path, PathBuf};

/// What `path` holds, as far as the ownership check is concerned.
pub(super) enum Existing {
	/// Nothing is there.
	Absent,
	/// A symbolic link. The write replaces the link itself and never follows it,
	/// so there is nothing to read.
	Link,
	/// A regular file. `None` when it could not be read as text within the size
	/// cap, which is treated like a file without a marker.
	File(Option<String>),
}

/// Inspect the entry at `path` without following a link, and read it only when
/// it is a regular file, through the capped reader.
///
/// Anything else (a FIFO, a device, a socket, a directory) is refused: opening
/// a FIFO for reading blocks until someone writes to it, and none of them is a
/// unit this tool wrote.
pub(super) fn inspect_existing(path: &Path) -> io::Result<Existing> {
	let meta = match path.symlink_metadata() {
		Ok(meta) => meta,
		Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Existing::Absent),
		Err(e) => return Err(e),
	};
	let kind = meta.file_type();
	if kind.is_symlink() {
		return Ok(Existing::Link);
	}
	if !kind.is_file() {
		return Err(io::Error::new(
			io::ErrorKind::InvalidInput,
			format!(
				"refusing to replace {}: it is not a regular file",
				path.display()
			),
		));
	}
	Ok(Existing::File(
		crate::filesystem::read_to_string_capped(path).ok(),
	))
}

/// Replace `path` with a private file holding `contents`.
///
/// The text goes to a new file next to the target, created exclusively (so it
/// can never be a pre-planted link or an existing file) with mode 0600 from the
/// start, and is then renamed over the target. The rename replaces whatever
/// entry is at the name, a link included, instead of writing through it, and the
/// target never exists with looser permissions or half written.
pub(super) fn write_private(path: &Path, contents: &str) -> io::Result<()> {
	use std::io::Write;

	let dir = path.parent().unwrap_or_else(|| Path::new("."));
	let name = path
		.file_name()
		.map(|n| n.to_string_lossy().into_owned())
		.unwrap_or_default();
	let mut attempt = 0u32;
	let (tmp, mut file) = loop {
		let tmp: PathBuf = dir.join(format!(".{name}.{}.{attempt}.tmp", std::process::id()));
		let mut options = std::fs::OpenOptions::new();
		options.write(true).create_new(true);
		#[cfg(unix)]
		{
			use std::os::unix::fs::OpenOptionsExt;
			options.mode(0o600);
		}
		match options.open(&tmp) {
			Ok(file) => break (tmp, file),
			Err(e) if e.kind() == io::ErrorKind::AlreadyExists && attempt < 16 => attempt += 1,
			Err(e) => return Err(e),
		}
	};
	let result = file
		.write_all(contents.as_bytes())
		.and_then(|()| file.sync_all())
		.and_then(|()| {
			drop(file);
			std::fs::rename(&tmp, path)
		});
	if result.is_err() {
		let _ = std::fs::remove_file(&tmp);
	}
	result
}
