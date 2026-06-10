//! `cp` command: copy files between a service container and the host.

use std::path::Path;

use bollard::body_full;
use bollard::query_parameters::{DownloadFromContainerOptions, UploadToContainerOptions};
use bytes::Bytes;
use flate2::write::GzEncoder;
use flate2::Compression;
use futures::StreamExt;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};

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

		let mut stream = self.docker.download_from_container(
			&container_name,
			Some(DownloadFromContainerOptions {
				path: container_path.to_string(),
			}),
		);

		let mut tar_bytes: Vec<u8> = Vec::new();
		while let Some(chunk) = stream.next().await {
			tar_bytes.extend_from_slice(&chunk?);
		}

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

		let cursor = std::io::Cursor::new(tar_bytes);
		let mut archive = tar::Archive::new(cursor);
		archive.unpack(&dst_path).map_err(ComposeError::Io)?;

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

		let tar_bytes = pack_path(src)?;

		self.docker
			.upload_to_container(
				&container_name,
				Some(UploadToContainerOptions {
					path: container_path.to_string(),
					..Default::default()
				}),
				body_full(Bytes::from(tar_bytes)),
			)
			.await?;

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
}
