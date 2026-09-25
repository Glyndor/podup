//! Apply `build.tags` aliases to a freshly built image.
//!
//! Split out of `service.rs` so the dispatching loop there stays under
//! the source-line limit. The companion to
//! [`super::Engine::build_service`]: called once a build has produced
//! its primary tag and needs every other `build.tags` entry attached
//! to the same image.

use crate::compose::types::BuildConfig;
use crate::error::{ComposeError, Result};
use crate::libpod::urlencoded;
use crate::libpod::API_PREFIX;

use super::Engine;

impl Engine {
	/// Apply any `build.tags` aliases to the freshly built image.
	///
	/// The primary `tag` is skipped: when no `image:` is set it is already
	/// `tags[0]`, which the build itself produced, so re-tagging it onto itself
	/// would be a no-op API call.
	pub(in crate::engine) async fn apply_extra_tags(
		&self,
		build: &BuildConfig,
		tag: &str,
	) -> Result<()> {
		for extra_tag in build.tags() {
			if extra_tag == tag {
				continue;
			}
			// The destination tag goes through the same docker.io
			// canonical form the compat build handler applied via
			// `NormalizeToDockerHub`. The libpod `/images/{}/tag`
			// endpoint accepts any name but stores the image under
			// whatever `repo:tag` it was given, so the only way to
			// keep an unqualified `proj-extra:1` on the docker.io
			// canonical form is to expand it here, before the POST.
			let normalized = self.normalize_image_reference(extra_tag).await?;
			let (repo, tag_str) = normalized
				.rsplit_once(':')
				.map(|(r, t)| (r.to_string(), t.to_string()))
				.unwrap_or_else(|| (normalized.clone(), "latest".to_string()));
			let encoded_tag = urlencoded(tag);
			let tag_path = format!(
				"{API_PREFIX}/images/{encoded_tag}/tag?repo={}&tag={}",
				urlencoded(&repo),
				urlencoded(&tag_str),
			);
			// Returning () here meant `build` could not report a failed tag at
			// all: it exited 0 with the requested tags missing.
			self.client
				.post_empty_ok(&tag_path)
				.await
				.map_err(ComposeError::Podman)?;
		}
		Ok(())
	}
}
