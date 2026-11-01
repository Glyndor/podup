use crate::engine::Engine;
use crate::error::{ComposeError, Result};
use crate::libpod::API_PREFIX;

impl Engine {
	/// True when a container with this exact name exists (any project). Used to
	/// refuse clobbering a pre-existing container on `run --name`.
	pub(in crate::backends::engine::lifecycle) async fn container_exists(
		&self,
		name: &str,
	) -> Result<bool> {
		let path = format!(
			"{API_PREFIX}/containers/{}/json",
			crate::libpod::urlencoded(name),
		);
		match self.client.get_json::<serde_json::Value>(&path).await {
			Ok(_) => Ok(true),
			Err(e) if e.is_status(404) => Ok(false),
			Err(e) => Err(ComposeError::Podman(e)),
		}
	}
}
