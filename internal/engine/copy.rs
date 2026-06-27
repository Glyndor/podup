//! `cp` command: copy files between a service container and the host.

use std::path::Path;

use bytes::Bytes;
use flate2::write::GzEncoder;
use flate2::Compression;
use http_body_util::{BodyExt, Limited};

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::urlencoded;
use crate::libpod::API_PREFIX;

use super::Engine;

/// Upper bound on a container→host `cp` archive buffered in memory. Without it a
/// hostile or huge container path would OOM the CLI. Generous (covers ordinary
/// file/dir copies); larger transfers should use `podman cp` directly.
const MAX_CP_ARCHIVE_BYTES: usize = 1024 * 1024 * 1024;

/// Options for [`Engine::cp_with_options`], mirroring `docker compose cp` flags.
#[derive(Default)]
pub struct CpOptions {
	/// 1-based replica index for a scaled service, `--index` (default: first).
	pub index: Option<u32>,
	/// Follow symlinks in the host source before packing, `-L/--follow-link`.
	pub follow_link: bool,
	/// Archive mode, `-a/--archive`. Accepted for command-line compatibility:
	/// under rootless Podman the original uid/gid cannot be restored, and
	/// container→host extraction always applies podup's security-hardened mode
	/// sanitization, so this flag has no effect on the copied bytes.
	pub archive: bool,
}

impl Engine {
	/// Copy between a service container and the local filesystem.
	///
	/// Either `src` or `dst` (but not both) must have the form `SERVICE:PATH`.
	/// The other side is a local path. `SERVICE:-` / `-:SERVICE` for stdin/stdout
	/// is not supported.
	pub async fn cp(&self, file: &ComposeFile, src: &str, dst: &str) -> Result<()> {
		self.cp_with_options(file, src, dst, CpOptions::default())
			.await
	}

	/// Copy with `docker compose cp` options: `--index` (target a specific
	/// replica), `-L/--follow-link` (follow host symlinks when uploading) and
	/// `-a/--archive` (accepted for compatibility — see [`CpOptions::archive`]).
	pub async fn cp_with_options(
		&self,
		file: &ComposeFile,
		src: &str,
		dst: &str,
		opts: CpOptions,
	) -> Result<()> {
		match (parse_endpoint(src), parse_endpoint(dst)) {
			(Some((service, container_path)), None) => {
				self.cp_from_container(file, service, container_path, Path::new(dst), &opts)
					.await
			}
			(None, Some((service, container_path))) => {
				self.cp_to_container(file, service, Path::new(src), container_path, &opts)
					.await
			}
			(Some(_), Some(_)) => Err(ComposeError::Unsupported(
				"cp: both src and dst cannot be SERVICE:PATH".into(),
			)),
			(None, None) => Err(ComposeError::Unsupported(
				"cp: one of src or dst must be SERVICE:PATH".into(),
			)),
		}
	}

	async fn cp_from_container(
		&self,
		file: &ComposeFile,
		service_name: &str,
		container_path: &str,
		dst: &Path,
		opts: &CpOptions,
	) -> Result<()> {
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;
		let container_name = self.replica_name_at(service_name, service, opts.index)?;

		let path = format!(
			"{API_PREFIX}/containers/{}/archive?path={}",
			urlencoded(&container_name),
			urlencoded(container_path),
		);
		let resp = self
			.client
			.get_stream(&path)
			.await
			.map_err(ComposeError::Podman)?;
		// Cap the buffered archive so a huge/hostile container path cannot OOM the
		// CLI (the streaming `get_stream` path bypasses the client's own cap).
		let tar_bytes = Limited::new(resp.into_body(), MAX_CP_ARCHIVE_BYTES)
			.collect()
			.await
			.map_err(|_| {
				ComposeError::Unsupported(format!(
					"cp: container archive exceeds {MAX_CP_ARCHIVE_BYTES} bytes; \
					 copy fewer files or use `podman cp` for very large transfers"
				))
			})?
			.to_bytes()
			.to_vec();

		let dst = dst.to_path_buf();
		tokio::task::spawn_blocking(move || extract_archive(&tar_bytes, &dst))
			.await
			.map_err(|e| ComposeError::Build(e.to_string()))??;

		Ok(())
	}

	async fn cp_to_container(
		&self,
		file: &ComposeFile,
		service_name: &str,
		src: &Path,
		container_path: &str,
		opts: &CpOptions,
	) -> Result<()> {
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;
		let container_name = self.replica_name_at(service_name, service, opts.index)?;

		// Match `docker cp` destination semantics. The libpod archive PUT extracts
		// the tar *at* a directory, so:
		//  - dest is an existing directory (or ends in `/`)  → copy the source in
		//    under its own name (PUT to the dest dir);
		//  - dest is anything else (a new name, or a file)   → rename the source to
		//    the dest's basename and PUT to the dest's parent.
		// Without this, `cp file svc:/path/newname` created `newname/` as a
		// directory holding the source instead of a file named `newname`.
		let stat_path = format!(
			"{API_PREFIX}/containers/{}/archive?path={}",
			urlencoded(&container_name),
			urlencoded(container_path),
		);
		let dest_is_dir = self.client.head_path_is_dir(&stat_path).await? == Some(true);

		let (extract_dir, rename) = if dest_is_dir || container_path.ends_with('/') {
			(container_path.trim_end_matches('/').to_string(), None)
		} else {
			let trimmed = container_path.trim_end_matches('/');
			let (parent, name) = trimmed.rsplit_once('/').unwrap_or(("", trimmed));
			let parent = if parent.is_empty() { "/" } else { parent };
			(parent.to_string(), Some(name.to_string()))
		};

		let src_buf = src.to_path_buf();
		let follow = opts.follow_link;
		let rename_for_pack = rename.clone();
		let tar_bytes = tokio::task::spawn_blocking(move || {
			pack_path(&src_buf, follow, rename_for_pack.as_deref())
		})
		.await
		.map_err(|e| ComposeError::Build(e.to_string()))??;

		let path = format!(
			"{API_PREFIX}/containers/{}/archive?path={}",
			urlencoded(&container_name),
			urlencoded(&extract_dir),
		);
		self.client
			.put_bytes_ok(&path, Bytes::from(tar_bytes), "application/x-tar")
			.await
			.map_err(ComposeError::Podman)?;

		Ok(())
	}
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_endpoint(s: &str) -> Option<(&str, &str)> {
	if s == "-" {
		return None;
	}
	// `SERVICE:PATH` — colon must not be the first character and path cannot be empty.
	let (svc, path) = s.split_once(':')?;
	if svc.is_empty() || path.is_empty() {
		return None;
	}
	// On Windows, an absolute path like `C:\path` has a single-char drive prefix —
	// treat those as local paths, not service endpoints. This must NOT apply on
	// Unix, where a one-character service name (`c:/path`) is perfectly valid and
	// would otherwise be rejected as a bogus "drive".
	#[cfg(windows)]
	if svc.len() == 1 && svc.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
		return None;
	}
	Some((svc, path))
}

fn pack_path(src: &Path, follow_link: bool, name_override: Option<&str>) -> Result<Vec<u8>> {
	let encoder = GzEncoder::new(Vec::new(), Compression::default());
	let mut tar = tar::Builder::new(encoder);
	// `-L/--follow-link`: archive the symlink target's contents instead of the
	// link itself.
	tar.follow_symlinks(follow_link);

	if src.is_dir() {
		// `name_override` renames the copied tree (rename-on-copy); otherwise it
		// keeps the source's own basename and lands inside the destination dir.
		let default = src.file_name().unwrap_or(std::ffi::OsStr::new("."));
		let name: &std::ffi::OsStr = name_override.map(std::ffi::OsStr::new).unwrap_or(default);
		tar.append_dir_all(name, src)
			.map_err(|e| ComposeError::Build(e.to_string()))?;
	} else {
		let default = src.file_name().unwrap_or(std::ffi::OsStr::new("file"));
		let name: &std::ffi::OsStr = name_override.map(std::ffi::OsStr::new).unwrap_or(default);
		tar.append_path_with_name(src, name)
			.map_err(|e| ComposeError::Build(e.to_string()))?;
	}

	let gz = tar
		.into_inner()
		.map_err(|e| ComposeError::Build(e.to_string()))?;
	gz.finish().map_err(|e| ComposeError::Build(e.to_string()))
}

/// Route a container archive to the host destination.
///
/// docker/podman `cp` semantics: when `dst` is an existing directory the archive
/// is extracted into it; otherwise `dst` names the target file and the archive
/// must contain a single regular file, whose **content** is written to exactly
/// `dst` — the daemon-supplied entry name is ignored. This matters for security:
/// a hostile image must not be able to choose the on-host filename (e.g. drop a
/// `.bashrc`/`authorized_keys` into the destination directory).
fn extract_archive(tar_bytes: &[u8], dst: &Path) -> Result<()> {
	if dst.is_dir() {
		return extract_tar_guarded(tar_bytes, dst);
	}
	write_single_entry_to(tar_bytes, dst)
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
	use super::parse_endpoint;

	#[test]
	fn parse_service_colon_path() {
		assert_eq!(parse_endpoint("web:/app/data"), Some(("web", "/app/data")));
	}

	#[test]
	fn parse_local_path_no_colon() {
		assert_eq!(parse_endpoint("/tmp/file.txt"), None);
	}

	#[test]
	fn parse_dash_is_local() {
		assert_eq!(parse_endpoint("-"), None);
	}

	#[cfg(windows)]
	#[test]
	fn parse_windows_drive_letter_is_local() {
		assert_eq!(parse_endpoint("C:\\Users\\foo"), None);
	}

	#[cfg(not(windows))]
	#[test]
	fn single_char_service_parses_on_unix() {
		// On Unix a one-character service name is valid; only Windows treats a
		// single-char prefix as a drive letter.
		assert_eq!(parse_endpoint("c:/tmp/file"), Some(("c", "/tmp/file")));
		assert_eq!(parse_endpoint("w:data"), Some(("w", "data")));
	}

	#[test]
	fn parse_empty_service_or_path() {
		assert_eq!(parse_endpoint(":path"), None);
		assert_eq!(parse_endpoint("svc:"), None);
	}

	#[cfg(windows)]
	#[test]
	fn parse_windows_drive_letter_forward_slash() {
		assert_eq!(parse_endpoint("C:/Users/foo"), None);
	}

	#[test]
	fn parse_service_with_relative_path() {
		assert_eq!(
			parse_endpoint("web:data/file.txt"),
			Some(("web", "data/file.txt"))
		);
	}

	#[test]
	fn parse_service_name_with_dots() {
		assert_eq!(
			parse_endpoint("my.service:/app/config"),
			Some(("my.service", "/app/config"))
		);
	}

	#[test]
	fn pack_path_single_file() {
		let dir = tempfile::tempdir().expect("tempdir");
		let file = dir.path().join("data.txt");
		std::fs::write(&file, b"hello").expect("write");
		let result = super::pack_path(&file, false, None);
		assert!(result.is_ok());
		let bytes = result.unwrap();
		assert!(!bytes.is_empty());
	}

	#[test]
	fn pack_path_directory() {
		let dir = tempfile::tempdir().expect("tempdir");
		let subdir = dir.path().join("mydir");
		std::fs::create_dir(&subdir).expect("mkdir");
		std::fs::write(subdir.join("a.txt"), b"aaa").expect("write");
		std::fs::write(subdir.join("b.txt"), b"bbb").expect("write");
		let result = super::pack_path(&subdir, false, None);
		assert!(result.is_ok());
		assert!(!result.unwrap().is_empty());
	}

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
	fn extract_archive_to_file_rejects_non_regular_entry() {
		// A single directory entry (not a regular file) against a file destination
		// is rejected — only a directory destination accepts a directory tree.
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
		assert!(super::extract_archive(&bytes, &dst).is_err());
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
