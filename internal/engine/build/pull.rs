//! Image pull from a registry (the non-build half of image acquisition).

use futures_util::StreamExt;
use tracing::{debug, info, warn};

use std::collections::HashSet;

use crate::compose::types::{ComposeFile, Service};
use crate::error::{ComposeError, Result};
use crate::libpod::types::image::ImagePullProgress;
use crate::libpod::{urlencoded, API_PREFIX};

use super::super::Engine;

/// Options for [`Engine::pull_services_with_options`], mirroring `docker
/// compose pull` flags. The `--policy` override is carried on the engine (see
/// [`Engine::with_up_overrides`]), not here.
#[derive(Default)]
pub struct PullOptions {
	/// Warn and continue instead of aborting on the first failure,
	/// `--ignore-pull-failures`.
	pub ignore_failures: bool,
	/// Also pull each named service's transitive `depends_on`, `--include-deps`.
	pub include_deps: bool,
}

impl Engine {
	/// Pull images for all services that declare an `image:` key, concurrently.
	pub async fn pull(&self, file: &ComposeFile) -> Result<()> {
		self.pull_services(file, &[]).await
	}

	/// Pull images for the named services (or every service when `services` is
	/// empty), matching `docker compose pull [SERVICE...]`.
	pub async fn pull_services(&self, file: &ComposeFile, services: &[String]) -> Result<()> {
		self.pull_services_with_options(file, services, PullOptions::default())
			.await
	}

	/// Pull service images with `docker compose pull` options:
	/// `--include-deps` (also pull each named service's transitive
	/// `depends_on`) and `--ignore-pull-failures` (warn and continue instead of
	/// aborting on the first failure). The `--policy` override is applied via
	/// the engine's pull-policy override (see [`Engine::with_up_overrides`]).
	pub async fn pull_services_with_options(
		&self,
		file: &ComposeFile,
		services: &[String],
		opts: PullOptions,
	) -> Result<()> {
		// `--include-deps` widens the explicit service list to its transitive
		// depends_on closure; an empty list already means "every service".
		let wanted: Option<HashSet<String>> = match (services.is_empty(), opts.include_deps) {
			(true, _) => None,
			(false, true) => Some(pull_dep_closure(file, services)),
			(false, false) => Some(services.iter().cloned().collect()),
		};

		let futs: Vec<_> = file
			.services
			.iter()
			.filter(|(name, s)| {
				s.image.is_some()
					&& wanted
						.as_ref()
						.is_none_or(|set| set.contains(name.as_str()))
			})
			.map(|(name, s)| async move {
				// The libpod pull stream reports failure as an in-band progress
				// line, so `pull_image` returns Ok even when the pull failed;
				// confirm the image actually landed in local storage.
				let _ = self.pull_image(s).await;
				let image = s.image.clone().unwrap_or_default();
				(
					name.clone(),
					image.clone(),
					self.image_present(&image).await,
				)
			})
			.collect();

		let results = futures_util::future::join_all(futs).await;
		for (name, image, present) in results {
			if present {
				continue;
			}
			if opts.ignore_failures {
				tracing::warn!("pull {name} ({image}) failed — ignored");
			} else {
				return Err(ComposeError::Build(format!(
					"failed to pull image {image} for service {name}"
				)));
			}
		}
		Ok(())
	}

	pub(in crate::engine) async fn pull_image(&self, service: &Service) -> Result<()> {
		let image = match &service.image {
			Some(img) => img.clone(),
			None => return Ok(()),
		};

		if self.quiet_pull {
			debug!("pulling {image}");
		} else {
			info!("pulling {image}");
		}

		// `up --pull <policy>` overrides the per-service `pull_policy`.
		let requested = self
			.pull_policy_override
			.as_deref()
			.or(service.pull_policy.as_deref());
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

	/// Whether an image reference is present in local storage. Used by the
	/// `pull` command to verify each pull actually landed (the streaming pull
	/// endpoint reports failures as in-band progress lines, not an HTTP error).
	async fn image_present(&self, image: &str) -> bool {
		let path = format!("{API_PREFIX}/images/{}/json", urlencoded(image));
		self.client
			.get_json::<crate::libpod::types::image::ImageInspect>(&path)
			.await
			.is_ok()
	}
}

/// The transitive `depends_on` closure of `services` (including the services
/// themselves), for `pull --include-deps`.
fn pull_dep_closure(file: &ComposeFile, services: &[String]) -> HashSet<String> {
	let mut set = HashSet::new();
	let mut stack: Vec<String> = services.to_vec();
	while let Some(name) = stack.pop() {
		if !set.insert(name.clone()) {
			continue;
		}
		if let Some(svc) = file.services.get(&name) {
			for dep in svc.depends_on.service_names() {
				if !set.contains(&dep) {
					stack.push(dep);
				}
			}
		}
	}
	set
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
	use super::{libpod_pull_policy, pull_dep_closure};

	#[test]
	fn dep_closure_includes_transitive_dependencies() {
		let file = crate::parse_str(
			"services:\n  web:\n    image: a\n    depends_on:\n      - api\n  api:\n    image: b\n    depends_on:\n      - db\n  db:\n    image: c\n  lone:\n    image: d\n",
		)
		.unwrap();
		let mut got: Vec<String> = pull_dep_closure(&file, &["web".to_string()])
			.into_iter()
			.collect();
		got.sort();
		assert_eq!(got, vec!["api", "db", "web"]);
	}

	#[test]
	fn dep_closure_of_leaf_is_just_itself() {
		let file = crate::parse_str("services:\n  db:\n    image: c\n").unwrap();
		let got: Vec<String> = pull_dep_closure(&file, &["db".to_string()])
			.into_iter()
			.collect();
		assert_eq!(got, vec!["db"]);
	}

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
