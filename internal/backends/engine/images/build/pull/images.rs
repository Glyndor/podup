use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, API_PREFIX};

use crate::engine::Engine;

impl Engine {
	/// Whether an image reference is present in local storage. Used by the
	/// `pull` command to verify each pull actually landed (the streaming pull
	/// endpoint reports failures as in-band progress lines, not an HTTP error),
	/// and by the `up` image-prefetch stage to skip a redundant pull request
	/// for an image a `missing`-policy service already has cached.
	pub(in crate::backends::engine) async fn image_present(&self, image: &str) -> bool {
		let path = format!("{API_PREFIX}/images/{}/json", urlencoded(image));
		self.client
			.get_json::<crate::libpod::types::image::ImageInspect>(&path)
			.await
			.is_ok()
	}

	/// The 64-hex ID the image reference resolves to in local storage right
	/// now, or `None` when no such image is present.
	///
	/// A name is a pointer and this is what it points at. `up` uses it to tell
	/// whether an existing container is still bound to the image its service
	/// resolves to, which the config hash cannot say: a rebuild, a pull or a
	/// `podman tag` moves the name and leaves the hash alone (#1620). A
	/// transport error is returned rather than folded into `None`, because the
	/// caller's fallback for "unknown" is to recreate, and a flaky socket must
	/// not silently turn every skip into a rebuild.
	pub(in crate::backends::engine) async fn image_id(
		&self,
		image: &str,
	) -> Result<Option<String>> {
		let path = format!("{API_PREFIX}/images/{}/json", urlencoded(image));
		match self
			.client
			.get_json::<crate::libpod::types::image::ImageInspect>(&path)
			.await
		{
			Ok(inspect) => Ok(Some(inspect.id)),
			Err(e) if e.is_status(404) => Ok(None),
			Err(e) => Err(ComposeError::Podman(e)),
		}
	}
}
