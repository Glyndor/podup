//! Volume creation and external-resource preflight.
//!
//! [`Engine::create_volumes`] pre-creates named volumes before containers start
//! and [`Engine::ensure_external_exists`] verifies declared external resources.
//! Secret/config materialisation lives in [`super::secrets`]; bind-string and
//! Mount-API helpers in [`super::volume_mounts`].

use std::collections::HashMap;

use tracing::info;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::types::volume::VolumeCreateOptions;
use crate::libpod::{urlencoded, API_PREFIX};

use super::Engine;

impl Engine {
	pub(super) async fn create_volumes(&self, file: &ComposeFile) -> Result<()> {
		for (name, config) in &file.volumes {
			let external = config.as_ref().and_then(|c| c.external).unwrap_or(false);
			if external {
				let external_name = config
					.as_ref()
					.and_then(|c| c.name.as_deref())
					.unwrap_or(name);
				self.ensure_external_exists("volume", "volumes", external_name)
					.await?;
				continue;
			}

			let volume_name = config
				.as_ref()
				.and_then(|c| c.name.as_deref())
				.map(|s| s.to_string())
				.unwrap_or_else(|| format!("{}_{}", self.project, name));

			let mut labels: HashMap<String, String> = config
				.as_ref()
				.map(|c| c.labels.to_map())
				.unwrap_or_default();
			labels.insert("podup.project".to_string(), self.project.clone());

			let driver = config
				.as_ref()
				.and_then(|c| c.driver.clone())
				.unwrap_or_else(|| "local".into());

			let driver_opts: HashMap<String, String> = config
				.as_ref()
				.map(|c| c.driver_opts.clone())
				.unwrap_or_default();

			let options = VolumeCreateOptions {
				name: Some(volume_name.clone()),
				driver: Some(driver),
				driver_opts,
				labels,
			};

			match self
				.client
				.post_json::<_, serde_json::Value>(
					&format!("{API_PREFIX}/volumes/create"),
					&options,
				)
				.await
			{
				Ok(_) => info!("created volume {volume_name}"),
				Err(ref e) if e.is_status(409) => {}
				Err(e) => return Err(ComposeError::Podman(e)),
			}
		}
		Ok(())
	}

	/// Verify an `external: true` resource (volume, network or secret) already
	/// exists on the host.
	///
	/// The compose spec requires podup to error when an external resource is
	/// declared but absent, rather than silently skipping it and letting
	/// containers fail later with an opaque mount/attach error.
	pub(super) async fn ensure_external_exists(
		&self,
		kind: &str,
		api_segment: &str,
		name: &str,
	) -> Result<()> {
		let path = format!("{API_PREFIX}/{api_segment}/{}/json", urlencoded(name));
		match self.client.get_json::<serde_json::Value>(&path).await {
			Ok(_) => Ok(()),
			Err(ref e) if e.is_status(404) => Err(ComposeError::ExternalNotFound(format!(
				"external {kind} \"{name}\" does not exist; create it before running"
			))),
			Err(e) => Err(ComposeError::Podman(e)),
		}
	}
}
