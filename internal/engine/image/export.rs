//! `commit` and `export`: snapshot a service container to an image, or stream
//! its filesystem out as a tar archive (`docker compose commit` / `export`).

use std::io::Write;
use std::path::PathBuf;

use http_body_util::BodyExt;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, API_PREFIX};

use super::super::Engine;

/// Split an `image` reference into `(repo, tag)`. A ':' after the last '/' is a
/// tag; otherwise it is part of a registry `host:port` and the whole string is
/// the repo. An omitted tag defaults to `latest`.
fn split_image_ref(image: &str) -> (&str, &str) {
	match image.rsplit_once(':') {
		Some((r, t)) if !t.contains('/') => (r, t),
		_ => (image, "latest"),
	}
}

/// Build the libpod `/commit` request path. `pause` quiesces the container
/// during the snapshot (docker default) for a consistent filesystem.
fn commit_path(container: &str, repo: &str, tag: &str, pause: bool) -> String {
	format!(
		"{API_PREFIX}/commit?container={}&repo={}&tag={}&pause={pause}",
		urlencoded(container),
		urlencoded(repo),
		urlencoded(tag),
	)
}

impl Engine {
	/// Commit a service container to a new image (`docker compose commit`).
	/// Targets the given replica (`index`, 1-based) or the first one. `image`
	/// is `repo[:tag]`; an omitted tag defaults to `latest`. `pause` quiesces the
	/// container during the snapshot for a consistent filesystem (docker default).
	pub async fn commit(
		&self,
		file: &ComposeFile,
		service_name: &str,
		image: &str,
		index: Option<u32>,
		pause: bool,
	) -> Result<()> {
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;
		let container = self.replica_name_at(service_name, service, index)?;

		let (repo, tag) = split_image_ref(image);
		let path = commit_path(&container, repo, tag, pause);
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

#[cfg(test)]
mod tests {
	use super::{commit_path, split_image_ref};

	#[test]
	fn image_ref_splits_repo_and_tag() {
		assert_eq!(split_image_ref("myrepo:v1"), ("myrepo", "v1"));
		// No tag defaults to latest.
		assert_eq!(split_image_ref("myrepo"), ("myrepo", "latest"));
		// A registry host:port is not a tag.
		assert_eq!(
			split_image_ref("localhost:5000/app"),
			("localhost:5000/app", "latest")
		);
		assert_eq!(
			split_image_ref("localhost:5000/app:v2"),
			("localhost:5000/app", "v2")
		);
	}

	#[test]
	fn commit_path_includes_pause_flag() {
		// Pausing (docker default) yields a consistent snapshot.
		let paused = commit_path("proj_web_1", "repo", "latest", true);
		assert!(paused.contains("container=proj_web_1"));
		assert!(paused.contains("repo=repo"));
		assert!(paused.contains("tag=latest"));
		assert!(paused.contains("pause=true"));
		// Opting out keeps the container live during commit.
		let live = commit_path("proj_web_1", "repo", "latest", false);
		assert!(live.contains("pause=false"));
	}
}
