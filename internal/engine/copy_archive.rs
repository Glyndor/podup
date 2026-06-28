//! Archive (un)packing helpers for `cp`: safely route a container→host tar to
//! the local filesystem, applying zip-slip and permission hardening.

use std::path::Path;

use crate::error::{ComposeError, Result};

/// Route a container archive to the host destination.
///
/// docker/podman `cp` semantics for a container→host extraction:
/// - `dst` is an existing directory: the archive is extracted into it (a source
///   directory lands under its own name).
/// - `dst` does not exist and the source is a single regular file: its
///   **content** is written to exactly `dst` — the daemon-supplied entry name is
///   ignored, so a hostile image cannot choose the on-host filename (e.g. drop a
///   `.bashrc`/`authorized_keys` into the destination directory).
/// - `dst` does not exist and the source is a directory: `dst` is created and the
///   source's *contents* are copied into it (matching `docker cp`).
pub(super) fn extract_archive(tar_bytes: &[u8], dst: &Path) -> Result<()> {
	if dst.is_dir() {
		return extract_tar_guarded(tar_bytes, dst);
	}
	if !archive_contains_dir(tar_bytes)? {
		// A single regular file (or, defensively, a name-only archive) is written
		// to exactly `dst`; `write_single_entry_to` still rejects a multi-entry
		// archive against a file destination.
		return write_single_entry_to(tar_bytes, dst);
	}
	// Directory source against an existing *file* destination: `create_dir_all`
	// would fail with a bare "File exists (os error 17)". Detect it up front and
	// emit a clear message naming the destination.
	if dst.exists() && !dst.is_dir() {
		return Err(ComposeError::Copy(format!(
			"cannot copy a directory onto existing file {}",
			dst.display()
		)));
	}
	// Directory source into a non-existent destination: create it and copy the
	// source's contents in. The libpod archive is tarred under the source's
	// basename, so extract through the zip-slip guard, then collapse that single
	// wrapper level to leave the contents directly under `dst`.
	std::fs::create_dir_all(dst).map_err(ComposeError::Io)?;
	extract_tar_guarded(tar_bytes, dst)?;
	flatten_single_wrapper_dir(dst)
}

/// True if any entry in `tar_bytes` is a directory — the signal that the source
/// of a container→host copy was a directory (libpod tars it under its basename),
/// as opposed to a single file.
fn archive_contains_dir(tar_bytes: &[u8]) -> Result<bool> {
	let cursor = std::io::Cursor::new(tar_bytes);
	let mut archive = tar::Archive::new(cursor);
	for entry in archive.entries().map_err(ComposeError::Io)? {
		let entry = entry.map_err(ComposeError::Io)?;
		if entry.header().entry_type() == tar::EntryType::Directory {
			return Ok(true);
		}
	}
	Ok(false)
}

/// Lift the contents of a single wrapper directory up into `dst`.
///
/// The libpod archive for `cp container:/srcdir` is tarred under the source's
/// basename (`srcdir/...`). When that lands in a freshly-created `dst`, docker
/// puts the source's *contents* directly in `dst`, so collapse the lone wrapper
/// level. A no-op unless `dst` holds exactly one entry and it is a directory.
fn flatten_single_wrapper_dir(dst: &Path) -> Result<()> {
	let mut children: Vec<std::path::PathBuf> = std::fs::read_dir(dst)
		.map_err(ComposeError::Io)?
		.filter_map(|e| e.ok().map(|e| e.path()))
		.collect();
	if children.len() != 1 || !children[0].is_dir() {
		return Ok(());
	}
	let wrapper = children.remove(0);
	for entry in std::fs::read_dir(&wrapper).map_err(ComposeError::Io)? {
		let from = entry.map_err(ComposeError::Io)?.path();
		let name = from
			.file_name()
			.ok_or_else(|| ComposeError::Build("cp: archive entry has no name".into()))?;
		std::fs::rename(&from, dst.join(name)).map_err(ComposeError::Io)?;
	}
	std::fs::remove_dir(&wrapper).map_err(ComposeError::Io)?;
	Ok(())
}

/// Extract a (plain, uncompressed) tar archive into `dst_dir`, refusing any
/// entry whose path would escape it (zip-slip) and stripping group/other-write
/// and setuid/setgid/sticky bits the (untrusted) container set on each entry.
///
/// Extract entry-by-entry rather than `archive.unpack`: a malicious or
/// compromised container can craft tar entries whose paths contain `..` or are
/// absolute, escaping `dst_dir` and overwriting host files. `unpack_in` refuses
/// such entries, returning `Ok(false)`; we turn that into a hard error so the
/// copy fails loudly instead of silently skipping data. Pure and synchronous so
/// the guard can be unit-tested without a container.
fn extract_tar_guarded(tar_bytes: &[u8], dst_dir: &Path) -> Result<()> {
	let cursor = std::io::Cursor::new(tar_bytes);
	let mut archive = tar::Archive::new(cursor);
	for entry in archive.entries().map_err(ComposeError::Io)? {
		let mut entry = entry.map_err(ComposeError::Io)?;
		let rel = entry.path().map(|p| p.into_owned()).ok();
		let mode = entry.header().mode().ok();
		if !entry.unpack_in(dst_dir).map_err(ComposeError::Io)? {
			let p = entry
				.path()
				.map(|p| p.display().to_string())
				.unwrap_or_else(|_| "<unprintable>".into());
			return Err(ComposeError::Build(format!(
				"cp: refusing archive entry that escapes destination: {p}"
			)));
		}
		if let (Some(rel), Some(mode)) = (rel, mode) {
			sanitize_extracted_mode(&dst_dir.join(rel), mode);
		}
	}
	Ok(())
}

/// Write the single regular-file entry of `tar_bytes` to exactly `dst`,
/// honouring the user's destination filename rather than the daemon's. Errors
/// if the archive is empty, holds more than one entry, or the entry is not a
/// regular file (those cases only make sense against a directory destination).
fn write_single_entry_to(tar_bytes: &[u8], dst: &Path) -> Result<()> {
	use std::io::Read;

	if let Some(parent) = dst.parent() {
		if !parent.as_os_str().is_empty() && !parent.exists() {
			std::fs::create_dir_all(parent).map_err(ComposeError::Io)?;
		}
	}

	let cursor = std::io::Cursor::new(tar_bytes);
	let mut archive = tar::Archive::new(cursor);
	let mut written = false;
	for entry in archive.entries().map_err(ComposeError::Io)? {
		let mut entry = entry.map_err(ComposeError::Io)?;
		if entry.header().entry_type() != tar::EntryType::Regular {
			return Err(ComposeError::Unsupported(format!(
				"cp: destination {} is not a directory but the source is not a single file",
				dst.display()
			)));
		}
		if written {
			return Err(ComposeError::Unsupported(format!(
				"cp: destination {} is not a directory but the source has multiple entries",
				dst.display()
			)));
		}
		let mode = entry.header().mode().ok();
		let mut buf = Vec::new();
		entry.read_to_end(&mut buf).map_err(ComposeError::Io)?;
		std::fs::write(dst, &buf).map_err(ComposeError::Io)?;
		if let Some(mode) = mode {
			sanitize_extracted_mode(dst, mode);
		}
		written = true;
	}
	if !written {
		return Err(ComposeError::Build(
			"cp: container archive was empty".into(),
		));
	}
	Ok(())
}

/// Strip group/other-write and setuid/setgid/sticky bits from a file extracted
/// from an untrusted container, keeping the owner and read/execute bits. No-op
/// on non-files (e.g. symlinks) and on non-unix platforms.
#[cfg(unix)]
fn sanitize_extracted_mode(path: &Path, mode: u32) {
	use std::os::unix::fs::PermissionsExt;
	let Ok(meta) = std::fs::symlink_metadata(path) else {
		return;
	};
	if !meta.is_file() {
		return;
	}
	let masked = mode & 0o7777 & !0o022 & !0o7000;
	if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(masked)) {
		tracing::warn!("cp: could not set permissions on {}: {e}", path.display());
	}
}

#[cfg(not(unix))]
fn sanitize_extracted_mode(_path: &Path, _mode: u32) {}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	/// Build an uncompressed tar archive with a single entry at `path`. The name
	/// is written straight into the GNU header so a hostile `..` path can be
	/// forged (the safe `set_path`/`append_data` helpers reject `..`).
	fn tar_bytes_with(path: &str, data: &[u8]) -> Vec<u8> {
		let mut header = tar::Header::new_gnu();
		header.set_size(data.len() as u64);
		header.set_mode(0o644);
		header.set_entry_type(tar::EntryType::Regular);
		let name = path.as_bytes();
		header.as_gnu_mut().expect("gnu header").name[..name.len()].copy_from_slice(name);
		header.set_cksum();
		let mut builder = tar::Builder::new(Vec::new());
		builder.append(&header, data).expect("append");
		builder.into_inner().expect("finish")
	}

	#[test]
	fn extract_tar_guarded_writes_benign_entry() {
		let dir = tempfile::tempdir().expect("tempdir");
		let bytes = tar_bytes_with("hello.txt", b"hi");
		super::extract_tar_guarded(&bytes, dir.path()).expect("extract");
		assert_eq!(
			std::fs::read(dir.path().join("hello.txt")).expect("read"),
			b"hi"
		);
	}

	#[test]
	fn extract_archive_to_file_honors_user_filename() {
		// dst is NOT a dir: the single entry's content must land at exactly `dst`,
		// ignoring the daemon-supplied entry name (a hostile image must not pick
		// the on-host filename).
		let dir = tempfile::tempdir().expect("tempdir");
		let dst = dir.path().join("myname.txt");
		let bytes = tar_bytes_with("evil-name", b"payload");
		super::extract_archive(&bytes, &dst).expect("extract");
		assert_eq!(std::fs::read(&dst).expect("read"), b"payload");
		assert!(
			!dir.path().join("evil-name").exists(),
			"daemon entry name must not be used as the on-host filename"
		);
	}

	#[test]
	fn extract_archive_to_file_rejects_multiple_entries() {
		let dir = tempfile::tempdir().expect("tempdir");
		let dst = dir.path().join("out.txt");
		let mut builder = tar::Builder::new(Vec::new());
		for n in ["a.txt", "b.txt"] {
			let mut h = tar::Header::new_gnu();
			h.set_size(1);
			h.set_mode(0o644);
			h.set_entry_type(tar::EntryType::Regular);
			h.set_path(n).expect("path");
			h.set_cksum();
			builder.append(&h, &b"x"[..]).expect("append");
		}
		let bytes = builder.into_inner().expect("finish");
		assert!(super::extract_archive(&bytes, &dst).is_err());
	}

	/// Build a directory archive shaped like libpod's `cp container:/srcdir`:
	/// a wrapper directory entry plus its children, all under the basename.
	fn tar_dir_with(wrapper: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
		let mut builder = tar::Builder::new(Vec::new());
		let mut d = tar::Header::new_gnu();
		d.set_size(0);
		d.set_mode(0o755);
		d.set_entry_type(tar::EntryType::Directory);
		d.set_path(format!("{wrapper}/")).expect("path");
		d.set_cksum();
		builder.append(&d, std::io::empty()).expect("dir");
		for (name, data) in files {
			let mut h = tar::Header::new_gnu();
			h.set_size(data.len() as u64);
			h.set_mode(0o644);
			h.set_entry_type(tar::EntryType::Regular);
			h.set_path(format!("{wrapper}/{name}")).expect("path");
			h.set_cksum();
			builder.append(&h, *data).expect("file");
		}
		builder.into_inner().expect("finish")
	}

	#[test]
	fn extract_dir_into_missing_dest_creates_and_flattens() {
		// dst does not exist and the source is a directory: dst is created and the
		// source's *contents* land directly in it (the wrapper level is collapsed),
		// matching `docker cp`.
		let dir = tempfile::tempdir().expect("tempdir");
		let dst = dir.path().join("newdir");
		let bytes = tar_dir_with("srcdir", &[("a.txt", b"aaa"), ("b.txt", b"bbb")]);
		super::extract_archive(&bytes, &dst).expect("extract");
		assert!(dst.is_dir());
		assert_eq!(std::fs::read(dst.join("a.txt")).expect("read"), b"aaa");
		assert_eq!(std::fs::read(dst.join("b.txt")).expect("read"), b"bbb");
		assert!(
			!dst.join("srcdir").exists(),
			"the wrapper directory level must be collapsed"
		);
	}

	#[test]
	fn extract_single_file_into_missing_dest_still_writes_exact_name() {
		// The single-file path is unchanged: content at exactly `dst`.
		let dir = tempfile::tempdir().expect("tempdir");
		let dst = dir.path().join("renamed.txt");
		let bytes = tar_bytes_with("original.txt", b"data");
		super::extract_archive(&bytes, &dst).expect("extract");
		assert_eq!(std::fs::read(&dst).expect("read"), b"data");
	}

	#[cfg(unix)]
	#[test]
	fn extract_strips_group_other_write_and_special_bits() {
		use std::os::unix::fs::PermissionsExt;
		let dir = tempfile::tempdir().expect("tempdir");
		// World-writable + setuid entry from an untrusted container.
		let mut h = tar::Header::new_gnu();
		h.set_size(2);
		h.set_mode(0o4777);
		h.set_entry_type(tar::EntryType::Regular);
		h.set_path("f").expect("path");
		h.set_cksum();
		let mut builder = tar::Builder::new(Vec::new());
		builder.append(&h, &b"hi"[..]).expect("append");
		let bytes = builder.into_inner().expect("finish");
		super::extract_tar_guarded(&bytes, dir.path()).expect("extract");
		let mode = std::fs::metadata(dir.path().join("f"))
			.expect("meta")
			.permissions()
			.mode() & 0o7777;
		assert_eq!(mode & 0o022, 0, "group/other write must be stripped");
		assert_eq!(mode & 0o7000, 0, "setuid/setgid/sticky must be stripped");
	}

	#[test]
	fn extract_tar_guarded_rejects_parent_traversal() {
		// A compromised container can return a tar whose entry escapes the
		// destination via `..`; the guard must refuse it and write nothing.
		let dir = tempfile::tempdir().expect("tempdir");
		let dst = dir.path().join("dest");
		std::fs::create_dir(&dst).expect("mkdir");
		let bytes = tar_bytes_with("../evil.txt", b"pwned");
		let err = super::extract_tar_guarded(&bytes, &dst).unwrap_err();
		assert!(
			format!("{err}").contains("escapes destination"),
			"expected a zip-slip refusal, got: {err}"
		);
		assert!(
			!dir.path().join("evil.txt").exists(),
			"traversal entry must not be written outside the destination"
		);
	}

	#[test]
	fn extract_archive_to_file_rejects_empty_archive() {
		// An empty tar (no entries) against a file destination is an error: there
		// is nothing to write to `dst`.
		let dir = tempfile::tempdir().expect("tempdir");
		let dst = dir.path().join("out.txt");
		let bytes = tar::Builder::new(Vec::new()).into_inner().expect("finish");
		let err = super::extract_archive(&bytes, &dst).unwrap_err();
		assert!(format!("{err}").contains("empty"), "got: {err}");
	}

	#[test]
	fn extract_archive_dir_source_creates_non_existent_dest() {
		// A directory source against a non-existent destination creates the
		// destination directory (matching `docker cp`) rather than erroring,
		// regardless of the destination's name.
		let dir = tempfile::tempdir().expect("tempdir");
		let dst = dir.path().join("out.txt");
		let mut h = tar::Header::new_gnu();
		h.set_size(0);
		h.set_mode(0o755);
		h.set_entry_type(tar::EntryType::Directory);
		h.set_path("subdir/").expect("path");
		h.set_cksum();
		let mut builder = tar::Builder::new(Vec::new());
		builder.append(&h, std::io::empty()).expect("append");
		let bytes = builder.into_inner().expect("finish");
		super::extract_archive(&bytes, &dst).expect("extract");
		assert!(dst.is_dir());
	}

	#[test]
	fn extract_dir_onto_existing_file_gives_clear_error() {
		// A directory source against an existing regular-file destination must fail
		// with a clear cp message naming the destination, not a raw "File exists".
		let dir = tempfile::tempdir().expect("tempdir");
		let dst = dir.path().join("afile");
		std::fs::write(&dst, b"existing").expect("write");
		let bytes = tar_dir_with("srcdir", &[("a.txt", b"aaa")]);
		let err = super::extract_archive(&bytes, &dst).unwrap_err();
		let msg = err.to_string();
		assert!(msg.contains("cp error"), "wrong category: {msg:?}");
		assert!(msg.contains("directory onto"), "got {msg:?}");
		assert!(!msg.contains("os error 17"), "raw errno leaked: {msg:?}");
	}

	#[test]
	fn extract_archive_to_file_creates_missing_parent() {
		// The destination's parent directory is created on demand so a fresh
		// `cp svc:/f ./new/dir/f` works without a pre-existing tree.
		let dir = tempfile::tempdir().expect("tempdir");
		let dst = dir.path().join("new").join("nested").join("file.txt");
		let bytes = tar_bytes_with("ignored-name", b"data");
		super::extract_archive(&bytes, &dst).expect("extract");
		assert_eq!(std::fs::read(&dst).expect("read"), b"data");
	}
}
