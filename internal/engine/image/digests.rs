//! `config --resolve-image-digests`: pin each service image to its registry
//! digest.
//!
//! Like `ls`, this is project-agnostic — it needs only a [`Client`] to inspect
//! images, not a full [`Engine`](crate::engine::Engine) — so it lives as a free function.

use crate::compose::types::ComposeFile;
use crate::error::Result;
use crate::libpod::types::image::ImageInspect;
use crate::libpod::{urlencoded, Client, API_PREFIX};

/// Return a copy of `file` with every service `image:` rewritten to its registry
/// digest (`repo@sha256:...`), matching `docker compose config
/// --resolve-image-digests`. An image with no registry digest in local storage
/// (e.g. built locally, or never pulled) is left unchanged with a warning.
pub async fn resolve_image_digests(client: &Client, file: &ComposeFile) -> Result<ComposeFile> {
	let mut out = file.clone();
	for (name, svc) in out.services.iter_mut() {
		let Some(image) = svc.image.clone() else {
			continue;
		};
		let path = format!("{API_PREFIX}/images/{}/json", urlencoded(&image));
		match client.get_json::<ImageInspect>(&path).await {
			Ok(info) => match info.repo_digests.into_iter().next() {
				Some(digest) => svc.image = Some(digest),
				None => tracing::warn!(
					"config --resolve-image-digests: no registry digest for {image} \
					 (service {name}); left unchanged"
				),
			},
			Err(e) => tracing::warn!(
				"config --resolve-image-digests: cannot inspect {image} (service {name}): {e}"
			),
		}
	}
	Ok(out)
}
