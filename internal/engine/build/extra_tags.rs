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
	/// `tag` is the un-normalised primary (what the print paths and the
	/// `up` board row carry); the comparison against each `build.tags`
	/// entry skips the alias that is the same un-normalised name.
	/// `wire_tag` is the docker.io canonical form the build produced
	/// (and the one `/libpod/images/{}/tag` actually has on disk), so
	/// it is what the source-side path of the POST carries.
	///
	/// Without the two-argument split the loop would either skip the
	/// wrong alias (comparing the normalised wire_tag against the
	/// un-normalised `build.tags` entry) or POST against an image the
	/// daemon does not have (the un-normalised primary as the source
	/// while the build landed under the normalised name).
	pub(in crate::engine) async fn apply_extra_tags(
		&self,
		build: &BuildConfig,
		tag: &str,
		wire_tag: &str,
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
			let encoded_source = urlencoded(wire_tag);
			let tag_path = format!(
				"{API_PREFIX}/images/{encoded_source}/tag?repo={}&tag={}",
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
