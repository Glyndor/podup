//! The healthcheck an image declares, read before a container is created.

use crate::engine::Engine;
use crate::libpod::types::image::{ImageHealthcheck, ImageInspect};
use crate::libpod::urlencoded;
use crate::libpod::API_PREFIX;

impl Engine {
	/// The `HEALTHCHECK` of `image`, or `None` when it has none or cannot be read.
	///
	/// A compose healthcheck that leaves a value out takes it from here (see
	/// `build_healthcheck_inheriting`). An image that is not there yet (404),
	/// or an answer that does not parse, leaves the specification's defaults
	/// in force, as before: creating the container reports a missing image
	/// better than this lookup does, so it never fails the create.
	pub(super) async fn image_healthcheck(&self, image: &str) -> Option<ImageHealthcheck> {
		let path = format!("{API_PREFIX}/images/{}/json", urlencoded(image));
		match self.client.get_json::<ImageInspect>(&path).await {
			Ok(inspect) => inspect.healthcheck,
			Err(e) => {
				tracing::debug!("could not read the healthcheck of image {image}: {e}");
				None
			}
		}
	}
}
