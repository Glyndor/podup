//! Image push to a registry (docker compose `push`).

use futures_util::StreamExt;
use tracing::{info, warn};

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::types::image::ImagePullProgress;
use crate::libpod::{urlencoded, API_PREFIX};

use super::super::Engine;

/// Options for [`Engine::push`], mirroring `docker compose push` (plus a Podman
/// `--tls-verify` escape hatch for insecure/local registries).
#[derive(Debug, Clone, Default)]
pub struct PushOptions {
	/// Continue with the remaining services after a push fails.
	pub ignore_failures: bool,
	/// Override TLS verification of the registry. `None` leaves Podman's default
	/// (verify on); `Some(false)` allows an insecure/HTTP registry.
	pub tls_verify: Option<bool>,
	/// Suppress the push progress output (`-q/--quiet`).
	pub quiet: bool,
}

impl Engine {
	/// Push each service's image to its registry. Services without an `image:`
	/// (build-only or imageless) are skipped. Registry credentials come from
	/// Podman's auth file (`podman login`), so no auth handling is needed here.
	pub async fn push(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		opts: PushOptions,
	) -> Result<()> {
		for svc in target_services {
			if !file.services.contains_key(svc) {
				return Err(ComposeError::ServiceNotFound(svc.clone()));
			}
		}

		for (name, service) in &file.services {
			if !target_services.is_empty() && !target_services.iter().any(|t| t == name) {
				continue;
			}
			let Some(image) = service.image.as_deref() else {
				tracing::debug!("{name}: no image to push, skipping");
				continue;
			};
			if let Err(e) = self.push_image(image, &opts).await {
				if opts.ignore_failures {
					warn!("push {image} failed (ignored): {e}");
				} else {
					return Err(e);
				}
			}
		}
		Ok(())
	}

	/// Push a single image ref and drain its progress stream, surfacing a
	/// mid-stream `error` line as a failure.
	async fn push_image(&self, image: &str, opts: &PushOptions) -> Result<()> {
		if opts.quiet {
			tracing::debug!("pushing {image}");
		} else {
			info!("pushing {image}");
		}
		let mut query = format!("destination={}", urlencoded(image));
		if let Some(tls) = opts.tls_verify {
			query.push_str(&format!("&tlsVerify={tls}"));
		}
		let path = format!("{API_PREFIX}/images/{}/push?{query}", urlencoded(image));

		let resp = self
			.client
			.post_empty_stream(&path)
			.await
			.map_err(ComposeError::Podman)?;
		let mut stream = crate::libpod::parse_json_lines::<ImagePullProgress>(resp.into_body());
		let mut stream_error: Option<String> = None;
		while let Some(result) = stream.next().await {
			match result {
				Ok(progress) => {
					if !progress.stream.is_empty() && !opts.quiet {
						info!("{}", progress.stream.trim_end());
					}
					if !progress.error.is_empty() {
						stream_error = Some(progress.error.clone());
					}
				}
				Err(e) => stream_error = Some(e.to_string()),
			}
		}
		match stream_error {
			Some(err) => Err(ComposeError::Build(format!("push {image}: {err}"))),
			None => {
				if opts.quiet {
					tracing::debug!("pushed {image}");
				} else {
					info!("pushed {image}");
				}
				Ok(())
			}
		}
	}
}
