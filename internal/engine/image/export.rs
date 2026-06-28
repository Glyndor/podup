//! `commit` and `export`: snapshot a service container to an image, or stream
//! its filesystem out as a tar archive (`docker compose commit` / `export`).

use std::io::Write;
use std::path::PathBuf;

use http_body_util::BodyExt;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, API_PREFIX};

use super::super::Engine;

impl Engine {
	/// Commit a service container to a new image (`docker compose commit`).
	/// Targets the given replica (`index`, 1-based) or the first one. `image`
	/// is `repo[:tag]`; an omitted tag defaults to `latest`.
	pub async fn commit(
		&self,
		file: &ComposeFile,
		service_name: &str,
		image: &str,
		index: Option<u32>,
	) -> Result<()> {
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;
		let container = self.replica_name_at(service_name, service, index)?;

		let (repo, tag) = split_image_ref(image)?;
		let path = format!(
			"{API_PREFIX}/commit?container={}&repo={}&tag={}",
			urlencoded(&container),
			urlencoded(repo),
			urlencoded(tag),
		);
		self.client
			.post_empty_ok(&path)
			.await
			.map_err(ComposeError::Podman)?;
		tracing::info!("committed {container} to {repo}:{tag}");
		Ok(())
	}

	/// Export a service container's filesystem as a tar stream (`docker compose
	/// export`), to `output` or stdout. Streamed chunk-by-chunk so a large
	/// rootfs is never fully buffered in memory.
	pub async fn export(
		&self,
		file: &ComposeFile,
		service_name: &str,
		output: Option<PathBuf>,
		index: Option<u32>,
	) -> Result<()> {
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;
		let container = self.replica_name_at(service_name, service, index)?;

		let path = format!("{API_PREFIX}/containers/{}/export", urlencoded(&container),);
		let resp = self
			.client
			.get_stream(&path)
			.await
			.map_err(ComposeError::Podman)?;

		let mut sink: Box<dyn Write> = match &output {
			Some(p) => Box::new(std::fs::File::create(p).map_err(|e| ComposeError::IoPath {
				path: p.display().to_string(),
				source: e,
			})?),
			None => Box::new(std::io::stdout().lock()),
		};
		let mut body = resp.into_body();
		while let Some(frame) = body.frame().await {
			let frame = frame.map_err(|e| ComposeError::Build(format!("export stream: {e}")))?;
			if let Ok(data) = frame.into_data() {
				sink.write_all(&data).map_err(|e| io_to_err(&output, e))?;
			}
		}
		sink.flush().map_err(|e| io_to_err(&output, e))?;
		Ok(())
	}
}

/// Map a write error to one that names the `-o` output path when present, so the
/// user learns which destination failed.
fn io_to_err(output: &Option<PathBuf>, e: std::io::Error) -> ComposeError {
	match output {
		Some(p) => ComposeError::IoPath {
			path: p.display().to_string(),
			source: e,
		},
		None => ComposeError::Io(e),
	}
}

/// Split a `commit` image reference into `(repo, tag)`, defaulting the tag to
/// `latest`. Rejects an empty reference or an empty repository (`""` or `:tag`),
/// which podman would otherwise accept and turn into a dangling `<none>` image.
/// Pure so it is unit-tested.
fn split_image_ref(image: &str) -> Result<(&str, &str)> {
	let (repo, tag) = match image.rsplit_once(':') {
		// A ':' after the last '/' is a tag; otherwise it's part of a registry
		// host:port and the whole string is the repo.
		Some((r, t)) if !t.contains('/') => (r, t),
		_ => (image, "latest"),
	};
	if repo.is_empty() {
		return Err(ComposeError::Unsupported(format!(
			"invalid image reference {image:?}: a non-empty repository name is required \
			 (e.g. myimage or myimage:tag)"
		)));
	}
	if tag.is_empty() {
		return Err(ComposeError::Unsupported(format!(
			"invalid image reference {image:?}: the tag after ':' must not be empty"
		)));
	}
	Ok((repo, tag))
}

#[cfg(test)]
mod tests {
	use super::split_image_ref;

	#[test]
	fn split_image_ref_defaults_tag() {
		assert_eq!(split_image_ref("myimage").unwrap(), ("myimage", "latest"));
		assert_eq!(split_image_ref("myimage:1.0").unwrap(), ("myimage", "1.0"));
	}

	#[test]
	fn split_image_ref_keeps_registry_port() {
		// A ':' that is part of a registry host:port is not a tag.
		assert_eq!(
			split_image_ref("registry:5000/app").unwrap(),
			("registry:5000/app", "latest")
		);
	}

	#[test]
	fn split_image_ref_rejects_empty_and_empty_repo() {
		assert!(split_image_ref("").is_err());
		assert!(split_image_ref(":tag").is_err());
		assert!(split_image_ref("repo:").is_err());
	}
}
