//! `cp` command: copy files between a service container and the host.

use std::path::Path;

use bytes::Bytes;
use flate2::write::GzEncoder;
use flate2::Compression;
use http_body_util::BodyExt;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::urlencoded;
use crate::libpod::API_PREFIX;

use super::Engine;

impl Engine {
	/// Copy between a service container and the local filesystem.
	///
	/// Either `src` or `dst` (but not both) must have the form `SERVICE:PATH`.
	/// The other side is a local path. `SERVICE:-` / `-:SERVICE` for stdin/stdout
	/// is not supported.
	pub async fn cp(&self, file: &ComposeFile, src: &str, dst: &str) -> Result<()> {
		match (parse_endpoint(src), parse_endpoint(dst)) {
			(Some((service, container_path)), None) => {
				self.cp_from_container(file, service, container_path, Path::new(dst))
					.await
			}
			(None, Some((service, container_path))) => {
				self.cp_to_container(file, service, Path::new(src), container_path)
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
	) -> Result<()> {
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;
		let container_name = self.container_name(service_name, service);

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
		let tar_bytes = resp
			.into_body()
			.collect()
			.await
			.map_err(|e| ComposeError::Podman(crate::libpod::PodmanError::Hyper(e)))?
			.to_bytes()
			.to_vec();

		let dst_path = if dst.is_dir() {
			dst.to_path_buf()
		} else if let Some(parent) = dst.parent() {
			if !parent.as_os_str().is_empty() && !parent.exists() {
				std::fs::create_dir_all(parent).map_err(ComposeError::Io)?;
			}
			parent.to_path_buf()
		} else {
			std::env::current_dir().map_err(ComposeError::Io)?
		};

		tokio::task::spawn_blocking(move || extract_tar_guarded(&tar_bytes, &dst_path))
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
	) -> Result<()> {
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;
		let container_name = self.container_name(service_name, service);

		let src_buf = src.to_path_buf();
		let tar_bytes = tokio::task::spawn_blocking(move || pack_path(&src_buf))
			.await
			.map_err(|e| ComposeError::Build(e.to_string()))??;

		let path = format!(
			"{API_PREFIX}/containers/{}/archive?path={}",
			urlencoded(&container_name),
			urlencoded(container_path),
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
	// Windows absolute paths like `C:\path` have a single-char drive prefix — treat
	// those as local paths, not as service endpoints.
	if svc.len() == 1 && svc.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
		return None;
	}
	Some((svc, path))
}

fn pack_path(src: &Path) -> Result<Vec<u8>> {
	let encoder = GzEncoder::new(Vec::new(), Compression::default());
	let mut tar = tar::Builder::new(encoder);

	if src.is_dir() {
		let name = src.file_name().unwrap_or(std::ffi::OsStr::new("."));
		tar.append_dir_all(name, src)
			.map_err(|e| ComposeError::Build(e.to_string()))?;
	} else {
		let name = src.file_name().unwrap_or(std::ffi::OsStr::new("file"));
		tar.append_path_with_name(src, name)
			.map_err(|e| ComposeError::Build(e.to_string()))?;
	}

	let gz = tar
		.into_inner()
		.map_err(|e| ComposeError::Build(e.to_string()))?;
	gz.finish().map_err(|e| ComposeError::Build(e.to_string()))
}

/// Extract a (plain, uncompressed) tar archive into `dst_dir`, refusing any
/// entry whose path would escape it (zip-slip).
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
		if !entry.unpack_in(dst_dir).map_err(ComposeError::Io)? {
			let p = entry
				.path()
				.map(|p| p.display().to_string())
				.unwrap_or_else(|_| "<unprintable>".into());
			return Err(ComposeError::Build(format!(
				"cp: refusing archive entry that escapes destination: {p}"
			)));
		}
	}
	Ok(())
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

	#[test]
	fn parse_windows_drive_letter_is_local() {
		assert_eq!(parse_endpoint("C:\\Users\\foo"), None);
	}

	#[test]
	fn parse_empty_service_or_path() {
		assert_eq!(parse_endpoint(":path"), None);
		assert_eq!(parse_endpoint("svc:"), None);
	}

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
		let result = super::pack_path(&file);
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
		let result = super::pack_path(&subdir);
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
}
