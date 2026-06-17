//! Image pull from a registry (the non-build half of image acquisition).

use futures_util::StreamExt;
use tracing::{debug, info, warn};

use crate::compose::types::Service;
use crate::error::{ComposeError, Result};
use crate::libpod::types::image::ImagePullProgress;
use crate::libpod::{urlencoded, API_PREFIX};

use super::super::Engine;

impl Engine {
	pub(in crate::engine) async fn pull_image(&self, service: &Service) -> Result<()> {
		let image = match &service.image {
			Some(img) => img.clone(),
			None => return Ok(()),
		};

		info!("pulling {image}");

		let requested = service.pull_policy.as_deref();
		let pull_policy = libpod_pull_policy(requested).unwrap_or_else(|| {
			warn!(
				"unknown pull_policy '{}', defaulting to 'missing'",
				requested.unwrap_or_default()
			);
			"missing"
		});
		let mut query = format!("reference={}&policy={}", urlencoded(&image), pull_policy);
		if let Some(platform) = &service.platform {
			query.push_str(&format!("&platform={}", urlencoded(platform)));
		}

		let path = format!("{API_PREFIX}/images/pull?{query}");
		let resp = self
			.client
			.post_empty_stream(&path)
			.await
			.map_err(ComposeError::Podman)?;
		let mut stream = crate::libpod::parse_json_lines::<ImagePullProgress>(resp.into_body());

		while let Some(result) = stream.next().await {
			match result {
				Ok(progress) => {
					if !progress.stream.is_empty() {
						debug!("{}", progress.stream.trim_end());
					}
					if !progress.error.is_empty() {
						warn!("pull error: {}", progress.error);
					}
				}
				Err(e) => warn!("pull warning: {e}"),
			}
		}

		Ok(())
	}
}

/// Map a compose `pull_policy:` value to the libpod images/pull `policy`
/// parameter. `if_not_present` is the spec alias for `missing`; `build` falls
/// back to `missing` here (its build behavior is handled by the caller). Returns
/// `None` for an unrecognized value so the caller can warn and default.
pub(in crate::engine) fn libpod_pull_policy(policy: Option<&str>) -> Option<&'static str> {
	match policy {
		Some("always") => Some("always"),
		Some("newer") => Some("newer"),
		Some("never") => Some("never"),
		None | Some("missing") | Some("if_not_present") | Some("build") => Some("missing"),
		Some(_) => None,
	}
}

#[cfg(test)]
mod tests {
	use super::libpod_pull_policy;

	#[test]
	fn pull_policy_maps_every_spec_value() {
		assert_eq!(libpod_pull_policy(Some("always")), Some("always"));
		assert_eq!(libpod_pull_policy(Some("newer")), Some("newer"));
		assert_eq!(libpod_pull_policy(Some("never")), Some("never"));
		assert_eq!(libpod_pull_policy(Some("missing")), Some("missing"));
		// `if_not_present` is the spec alias for `missing`.
		assert_eq!(libpod_pull_policy(Some("if_not_present")), Some("missing"));
		assert_eq!(libpod_pull_policy(Some("build")), Some("missing"));
		assert_eq!(libpod_pull_policy(None), Some("missing"));
		// Unknown values are reported (None) so the caller warns.
		assert_eq!(libpod_pull_policy(Some("bogus")), None);
	}
}
