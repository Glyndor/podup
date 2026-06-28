//! `commit` and `export`: snapshot a service container to an image, or stream
//! its filesystem out as a tar archive (`docker compose commit` / `export`).

use std::io::Write;
use std::path::PathBuf;

use http_body_util::BodyExt;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, API_PREFIX};

use super::super::Engine;

/// Options for [`Engine::commit_with_options`], mirroring `docker compose commit`
/// (`-m/--message`, `-a/--author`, `-p/--pause`, `-c/--change`). Kept off the
/// frozen `commit` signature so the 1.0 library API stays stable.
#[derive(Debug, Clone, Default)]
pub struct CommitOptions {
	/// Commit message recorded on the new image (`-m/--message`).
	pub message: Option<String>,
	/// Author recorded on the new image (`-a/--author`).
	pub author: Option<String>,
	/// Pause the container during the commit (`-p/--pause`); `None` leaves
	/// Podman's default (pause on).
	pub pause: Option<bool>,
	/// Dockerfile instructions to apply to the committed image (`-c/--change`).
	pub changes: Vec<String>,
}

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
		self.commit_with_options(file, service_name, image, index, CommitOptions::default())
			.await
	}

	/// Commit a service container with `docker compose commit`-style options
	/// (message, author, pause, change).
	pub async fn commit_with_options(
		&self,
		file: &ComposeFile,
		service_name: &str,
		image: &str,
		index: Option<u32>,
		opts: CommitOptions,
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
		let mut path = format!(
			"{API_PREFIX}/commit?container={}&repo={}&tag={}",
			urlencoded(&container),
			urlencoded(repo),
			urlencoded(tag),
		);
		if let Some(message) = &opts.message {
			path.push_str(&format!("&comment={}", urlencoded(message)));
		}
		if let Some(author) = &opts.author {
			path.push_str(&format!("&author={}", urlencoded(author)));
		}
		if let Some(pause) = opts.pause {
			path.push_str(&format!("&pause={pause}"));
		}
		for change in &opts.changes {
			path.push_str(&format!("&changes={}", urlencoded(change)));
		}
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

		// `-o -` streams to stdout (the coreutils/docker dash convention) rather
		// than creating a file literally named `-`.
		let to_stdout = output.is_none() || output.as_deref() == Some(std::path::Path::new("-"));
		let mut sink: Box<dyn Write> = if to_stdout {
			Box::new(std::io::stdout().lock())
		} else {
			Box::new(std::fs::File::create(output.as_ref().unwrap()).map_err(ComposeError::Io)?)
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
