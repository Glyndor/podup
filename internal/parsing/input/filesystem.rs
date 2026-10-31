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
	reject_non_regular(path)?;
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

/// [`read_capped`] with a limit of the caller's choosing that also returns the
/// permission bits of the file it read, taken from the same open handle as the bytes. The two cannot describe
/// different files, whatever happens to the name afterwards.
///
/// The bits are the file's own read and write bits (`0o666` of its mode); where
/// a file has no mode, the Compose Specification's `0o444`.
pub(crate) fn read_capped_with_mode(
	path: impl AsRef<Path>,
	max: u64,
) -> io::Result<(Vec<u8>, u32)> {
	let path = path.as_ref();
	reject_non_regular(path)?;
	let file = open_regular(path)?;
	let mode = mode_bits(&file.metadata()?);
	let mut buf = Vec::new();
	let read = file.take(max + 1).read_to_end(&mut buf)?;
	if read as u64 > max {
		return Err(io::Error::new(
			io::ErrorKind::InvalidData,
			format!("{} is larger than the {max} byte limit", path.display()),
		));
	}
	Ok((buf, mode))
}

/// The read and write permission bits of a file, which is what a secret or a
/// config is mounted with; `0o444` where a file has no mode.
pub(crate) fn mode_bits(metadata: &std::fs::Metadata) -> u32 {
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		metadata.permissions().mode() & 0o666
	}
	#[cfg(not(unix))]
	{
		let _ = metadata;
		0o444
	}
}

/// Open `path` and confirm, from the handle, that it is a regular file. On Unix
/// the open cannot block on a pipe that was put under the name after the check
/// of its type, and does not follow a link, since the type was checked without
/// following one. The check of the type before the open gives the better message;
/// this is the one that holds.
fn open_regular(path: &Path) -> io::Result<std::fs::File> {
	#[cfg(unix)]
	let file = {
		use std::os::unix::fs::OpenOptionsExt;
		std::fs::OpenOptions::new()
			.read(true)
			.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
			.open(path)?
	};
	#[cfg(not(unix))]
	let file = std::fs::File::open(path)?;
	if !file.metadata()?.is_file() {
		return Err(io::Error::new(
			io::ErrorKind::InvalidInput,
			format!("{} is not a regular file", path.display()),
		));
	}
	Ok(file)
}

fn read_capped_with(path: &Path, max: u64) -> io::Result<Vec<u8>> {
	reject_non_regular(path)?;
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

/// Refuse to read a path whose file type would block rather than return data.
///
/// A FIFO (named pipe) opened with the default flags blocks in the kernel
/// until a writer opens the other end. With no writer (or a writer that
/// never closes), `File::open` and the follow-up `read` both hang. `env_file:`
/// entries are configuration the operator wrote into a compose file, so the
/// only way a FIFO shows up here is a mistake (typo, accidentally pointing
/// at a leftover `/tmp/some.fifo`); rather than wedge the parser, refuse
/// the read with an actionable error before the open. The same goes for the
/// character/block device kinds, where reading `/dev/zero` would otherwise
/// spin until the cap. `symlink_metadata` rather than `metadata` keeps a
/// planted symlink at `path` from quietly reverting to the target type.
#[cfg(unix)]
fn reject_non_regular(path: &Path) -> io::Result<()> {
	use std::os::unix::fs::FileTypeExt;
	let meta = match path.symlink_metadata() {
		Ok(m) => m,
		// The follow-up `File::open` will report NotFound with the same path;
		// fall through to that so the error message stays consistent.
		Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
		Err(e) => return Err(e),
	};
	let ty = meta.file_type();
	if ty.is_file() {
		return Ok(());
	}
	let kind_label = if ty.is_fifo() {
		"FIFO (named pipe)"
	} else if ty.is_char_device() {
		"character device"
	} else if ty.is_block_device() {
		"block device"
	} else if ty.is_socket() {
		"socket"
	} else if ty.is_dir() {
		"directory"
	} else {
		"non-regular file"
	};
	Err(io::Error::new(
		io::ErrorKind::InvalidInput,
		format!(
			"refusing to read {kind_label} {}: would block forever",
			path.display()
		),
	))
}

/// Non-Unix: keep the old behaviour. `FileType` only knows about files and
/// directories in the platform-neutral surface, and a FIFO on Windows is not
/// reachable through the env_file path anyway.
#[cfg(not(unix))]
fn reject_non_regular(_path: &Path) -> io::Result<()> {
	Ok(())
}

#[cfg(test)]
#[path = "filesystem_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "filesystem_special_tests.rs"]
mod special_tests;
