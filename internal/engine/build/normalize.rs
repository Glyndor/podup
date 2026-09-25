//! Image-name normalisation for the libpod build path.
//!
//! The docker-compat build handler applied
//! [`NormalizeToDockerHub`](https://github.com/containers/podman/blob/v5.7.0/pkg/api/handlers/utils/images.go)
//! to every tag it forwarded to `POST /libpod/build` and
//! `POST /libpod/images/{}/tag`. On the libpod path that helper
//! short-circuits and returns the input unchanged, so the canonical
//! form the user used to see (a `docker.io/library/...` prefix on
//! every unqualified name) was lost. `podup build` that previously
//! produced `docker.io/library/proj-app:latest` started producing
//! `localhost/proj-app:latest` instead. Every build left a second
//! copy of the image behind, and `ps`/`images`/`events` listed the
//! wrong one.
//!
//! This module restores the helper's two-step contract with the
//! [`Engine::normalize_image_reference`] entry point:
//!
//! 1. Look the name up in local storage via
//!    `GET /libpod/images/{name}/json`. When the input resolves to
//!    a local image, return its canonical `RepoTags` entry that
//!    matches the input's tag (the upstream helper returns the
//!    candidate resolved by the daemon, which carries the same
//!    shape).
//! 2. Otherwise apply the pure normalisation rule
//!    ([`super::super::super::libpod::normalize::normalize_docker_reference`]).
//!
//! A pure helper sits next to the wire shape so the unit test can
//! pin the rule without a live socket. The wire-shape lookup is
//! async because it has to.

use crate::error::{ComposeError, Result};
use crate::libpod::types::image::ImageInspect;
use crate::libpod::{normalize::normalize_docker_reference, urlencoded, API_PREFIX};

use super::Engine;

impl Engine {
	/// Resolve `name` to the docker.io canonical form, falling back to the
	/// pure normalisation rule when no local image matches.
	///
	/// See module docs for why the libpod build path needs this: the
	/// docker-compat handler did it through `NormalizeToDockerHub`,
	/// which is gated by `IsLibpodRequest` and short-circuits on
	/// the libpod path.
	pub(in crate::engine) async fn normalize_image_reference(&self, name: &str) -> Result<String> {
		let path = format!("{API_PREFIX}/images/{}/json", urlencoded(name));
		match self.client.get_json::<ImageInspect>(&path).await {
			Ok(inspect) => {
				if let Some(matched) = match_repo_tag(&inspect.repo_tags, name) {
					return Ok(matched.to_string());
				}
			}
			Err(e) if e.is_status(404) => {}
			Err(e) => return Err(ComposeError::Podman(e)),
		}
		Ok(normalize_docker_reference(name))
	}
}

/// Pick the canonical `RepoTags` entry that matches `input`.
///
/// The daemon's `LookupImage` resolves a short name through
/// `registries.conf` and returns the canonical form. When that form
/// matches one of the local image's `RepoTags`, that entry is the
/// canonical name podup has to keep: the user tagged it themselves
/// and a fresh normalisation would land on a different name.
///
/// "Matches" means: same tag suffix when the input carries one, or
/// any entry when the input is a bare name. The first hit wins so
/// the result is deterministic across runs on the same local
/// storage state.
fn match_repo_tag<'a>(repo_tags: &'a [String], input: &str) -> Option<&'a str> {
	let wanted_suffix = input.split_once(':').map(|(_, tag)| tag);
	for tag in repo_tags {
		if let Some(suffix) = wanted_suffix {
			if tag.ends_with(&format!(":{suffix}")) {
				return Some(tag.as_str());
			}
		} else if !tag.contains(':') {
			return Some(tag.as_str());
		}
	}
	repo_tags.first().map(String::as_str)
}

#[cfg(test)]
#[path = "normalize_tests.rs"]
mod tests;
