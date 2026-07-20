//! Deciding which image a service runs, and getting it there.
//!
//! Split out of `mod.rs` to keep that file within the source line limit.

use crate::compose::types::{ComposeFile, Service};
use crate::error::Result;

use super::Engine;

impl Engine {
	/// The image tag a service resolves to: its explicit `image:` when set,
	/// otherwise the tag its build produces (`build.tags[0]`, else the
	/// project-scoped `{project}-{service}:latest`).
	///
	/// This is the name `up` checks for presence and the name `down --rmi local`
	/// removes, so both must agree on it — they used to compute it separately.
	pub(super) fn service_image_tag(&self, name: &str, service: &Service) -> String {
		match &service.image {
			Some(image) => image.clone(),
			None => crate::engine::build::primary_build_tag(
				&self.project,
				name,
				None,
				service.build.as_ref().map(|b| b.tags()).unwrap_or(&[]),
			),
		}
	}

	/// Make the service's image available before its containers are created:
	/// build it, pull it, or leave the local one alone.
	pub(super) async fn acquire_service_image(
		&self,
		name: &str,
		service: &Service,
		file: &ComposeFile,
	) -> Result<()> {
		// `up --pull <policy>` overrides the per-service `pull_policy`; `--no-build`
		// suppresses building even for services with a `build:` section (they fall
		// back to pulling/using an existing image).
		let policy = self
			.pull_policy_override
			.as_deref()
			.or(service.pull_policy.as_deref())
			.unwrap_or("missing");
		// Build on `up` only when the service's image is not already there, which
		// is what docker compose does: `up` converges on the declared state and
		// `--build` is the flag that forces a rebuild.
		//
		// Building unconditionally was worse than redundant. The rebuild runs
		// *with* the cache, so it can resolve to an older layer chain and retag
		// the image backwards — silently undoing a `podup build --no-cache` that
		// just ran. `build --no-cache && up -d`, the ordinary deploy shape, would
		// start the previous image. It also made `--build` look like a no-op,
		// since the default already always built.
		//
		// `--build` is handled before this by an explicit `build_all`, so a forced
		// rebuild has already happened and the image is present by the time we get
		// here.
		let needs_build = if service.build.is_some() && !self.no_build {
			!self
				.image_present(&self.service_image_tag(name, service))
				.await
		} else {
			false
		};
		match (needs_build, policy) {
			(true, _) => {
				self.build_service(name, service, file, &crate::engine::BuildOptions::default())
					.await?
			}
			// A service with a `build:` whose image is already present needs no
			// pull either — the local tag is the declared state.
			(false, _) if service.build.is_some() => {}
			(false, "never") => {}
			(false, _) => self.pull_image(service).await?,
		}
		Ok(())
	}
}
