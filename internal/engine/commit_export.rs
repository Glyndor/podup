//! `commit` and `export`: snapshot a service container to an image, or stream
//! its filesystem out as a tar archive (`docker compose commit` / `export`).

use std::io::Write;
use std::path::PathBuf;

use http_body_util::BodyExt;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, API_PREFIX};

use super::Engine;

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

		let (repo, tag) = match image.rsplit_once(':') {
			// A ':' after the last '/' is a tag; otherwise it's part of a registry
			// host:port and the whole string is the repo.
			Some((r, t)) if !t.contains('/') => (r, t),
			_ => (image, "latest"),
		};
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
			Some(p) => Box::new(std::fs::File::create(p).map_err(ComposeError::Io)?),
			None => Box::new(std::io::stdout().lock()),
		};
		let mut body = resp.into_body();
		while let Some(frame) = body.frame().await {
			let frame = frame.map_err(|e| ComposeError::Build(format!("export stream: {e}")))?;
			if let Ok(data) = frame.into_data() {
				sink.write_all(&data).map_err(ComposeError::Io)?;
			}
		}
		sink.flush().map_err(ComposeError::Io)?;
		Ok(())
	}
}
