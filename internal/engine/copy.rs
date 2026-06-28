//! `cp` command: copy files between a service container and the host.

use std::path::Path;

use bytes::Bytes;
use http_body_util::{BodyExt, Limited};

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::urlencoded;
use crate::libpod::API_PREFIX;

use super::Engine;

mod archive;

use archive::{extract_archive, pack_path};

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
		// Reject the explicitly-unsupported endpoint forms (`-` for stdin/stdout,
		// and a `SERVICE:` with an empty container path) with a clear message
		// before they silently fall through to a local file literally named `-`
		// or `SERVICE:`.
		check_endpoint(src)?;
		check_endpoint(dst)?;
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

		// Validate the extraction directory exists and is itself a directory before
		// PUTting the archive. Without this, libpod silently auto-creates a missing
		// parent chain (diverging from docker/podman `cp`, which error with "no such
		// directory"); and when a path component is a regular file the archive PUT
		// never gets a response, blocking the full READ_TIMEOUT window instead of
		// failing fast.
		let extract_stat_path = format!(
			"{API_PREFIX}/containers/{}/archive?path={}",
			urlencoded(&container_name),
			urlencoded(&extract_dir),
		);
		match self.client.head_path_is_dir(&extract_stat_path).await? {
			Some(true) => {}
			Some(false) => {
				return Err(ComposeError::Copy(format!(
					"cp: not a directory: {extract_dir}"
				)));
			}
			None => {
				return Err(ComposeError::Copy(format!(
					"cp: no such directory: {extract_dir}"
				)));
			}
		}

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

/// Reject the `cp` endpoint forms podup explicitly does not support, with a
/// clear diagnostic rather than letting them fall through to a local path that
/// happens to be named `-` or `SERVICE:`.
///
/// - `-` (stdin/stdout streaming) is not implemented.
/// - `SERVICE:` (a colon with an empty container path) is a malformed reference.
///
/// A plain local path (no colon, or a colon that is part of an ordinary host
/// path / Windows drive) is left to [`parse_endpoint`].
fn check_endpoint(s: &str) -> Result<()> {
	if s == "-" {
		return Err(ComposeError::Unsupported(
			"cp: stdin/stdout ('-') is not supported".into(),
		));
	}
	if let Some((svc, path)) = s.split_once(':') {
		// A Windows drive letter (`C:\...`) is a local path, not a service ref.
		#[cfg(windows)]
		if svc.len() == 1 && svc.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
			return Ok(());
		}
		if !svc.is_empty() && path.is_empty() {
			return Err(ComposeError::Copy(format!(
				"cp: empty container path in '{s}' (expected SERVICE:PATH)"
			)));
		}
	}
	Ok(())
}

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
	fn check_endpoint_rejects_dash() {
		let err = super::check_endpoint("-").unwrap_err();
		assert!(format!("{err}").contains("stdin/stdout"), "got: {err}");
	}

	#[test]
	fn check_endpoint_rejects_empty_container_path() {
		let err = super::check_endpoint("web:").unwrap_err();
		assert!(
			format!("{err}").contains("empty container path"),
			"got: {err}"
		);
	}

	#[test]
	fn check_endpoint_allows_normal_forms() {
		// A plain local path, a proper SERVICE:PATH, and a relative host path are
		// all fine (validation only rejects `-` and `SERVICE:`).
		assert!(super::check_endpoint("/tmp/file").is_ok());
		assert!(super::check_endpoint("web:/app/data").is_ok());
		assert!(super::check_endpoint("./local").is_ok());
	}
}
