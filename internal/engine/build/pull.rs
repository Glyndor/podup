//! Image pull from a registry (the non-build half of image acquisition).

use futures_util::StreamExt;
use tracing::{debug, warn};

use std::collections::{HashMap, HashSet};

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

/// Upper bound on how many distinct images a standalone `pull` fetches
/// concurrently. Mirrors the lifecycle level fan-out's own concurrency cap: a
/// compose file with many distinct images must not open an unbounded number
/// of simultaneous pull streams against the Podman socket.
const MAX_PULL_CONCURRENCY: usize = 16;

/// Run `futs` concurrently, capped at `limit` in flight at once. Unlike the
/// lifecycle fan-out's `join_bounded`, callers here have no use for
/// input-order results (the outcomes are reduced into an image-keyed map
/// right after), so this stays a plain bounded join.
async fn bounded_join_all<F, T>(futs: impl IntoIterator<Item = F>, limit: usize) -> Vec<T>
where
	F: std::future::Future<Output = T>,
{
	futures_util::stream::iter(futs)
		.buffer_unordered(limit)
		.collect()
		.await
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
	///
	/// Services sharing an image reference pull it once, not once per service:
	/// the actual pull is deduplicated by image, dispatched with bounded
	/// concurrency, and each service still gets its own present/error report
	/// derived from its image's single shared outcome.
	pub async fn pull_services_with_options(
		&self,
		file: &ComposeFile,
		services: &[String],
		opts: PullOptions,
	) -> Result<()> {
		// Reject unknown service names up front, matching `docker compose pull`
		// (and `logs`), rather than silently doing nothing.
		for name in services {
			if !file.services.contains_key(name) {
				return Err(ComposeError::ServiceNotFound(name.clone()));
			}
		}

		// `--include-deps` widens the explicit service list to its transitive
		// depends_on closure; an empty list already means "every service".
		let wanted: Option<HashSet<String>> = match (services.is_empty(), opts.include_deps) {
			(true, _) => None,
			(false, true) => Some(pull_dep_closure(file, services)),
			(false, false) => Some(services.iter().cloned().collect()),
		};

		// Every service this pull pass covers, in file order — kept so the
		// per-service reporting loop below stays deterministic — paired with
		// its own service config (needed to report `name`/`image` even though
		// the actual pull below runs once per unique image, not once here).
		let candidates: Vec<(&str, &Service)> = file
			.services
			.iter()
			.filter(|(name, s)| {
				s.image.is_some()
					&& wanted
						.as_ref()
						.is_none_or(|set| set.contains(name.as_str()))
			})
			.map(|(name, s)| (name.as_str(), s))
			.collect();

		// Dedup by image reference: 50 services on one image must issue one
		// pull, not 50. One representative service per unique image is enough
		// to issue it (only `image`/`pull_policy`/`platform` matter to the pull
		// itself).
		let mut representative: HashMap<&str, &Service> = HashMap::new();
		for (_, service) in &candidates {
			let image = service.image.as_deref().unwrap_or_default();
			representative.entry(image).or_insert(service);
		}

		// Pull each unique image once, bounded, and record its outcome — the
		// same present/error pair the per-service loop used to compute for
		// itself, now shared by every service that names the same image.
		let futs = representative
			.into_iter()
			.map(|(image, service)| async move {
				// The libpod pull stream reports failure as an in-band progress
				// line, so `pull_image` returns Ok even when the pull failed;
				// confirm the image actually landed in local storage. Keep the
				// real transport error (e.g. socket unreachable) so a failed pull
				// surfaces the underlying cause rather than a generic message.
				let pull_err = self.pull_image(service).await.err().map(|e| e.to_string());
				let present = self.image_present(image).await;
				(image.to_string(), present, pull_err)
			});
		let outcomes: HashMap<String, (bool, Option<String>)> =
			bounded_join_all(futs, MAX_PULL_CONCURRENCY)
				.await
				.into_iter()
				.map(|(image, present, err)| (image, (present, err)))
				.collect();

		for (name, service) in candidates {
			let image = service.image.as_deref().unwrap_or_default();
			let (present, pull_err) = outcomes.get(image).cloned().unwrap_or((false, None));
			if present {
				continue;
			}
			if opts.ignore_failures {
				match &pull_err {
					Some(e) => tracing::warn!("pull {name} ({image}) failed — ignored: {e}"),
					None => tracing::warn!("pull {name} ({image}) failed — ignored"),
				}
			} else {
				let detail = pull_err.map(|e| format!(": {e}")).unwrap_or_default();
				return Err(ComposeError::Build(format!(
					"failed to pull image {image} for service {name}{detail}"
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

		// Progress goes to stderr so it shows at default verbosity (the non-watch
		// log floor is WARN, so info!/debug! would print nothing) and `--quiet`
		// actually suppresses it, matching `docker compose pull`.
		if self.quiet_pull {
			debug!("pulling {image}");
		} else {
			eprintln!("Pulling {image}");
		}

		// `up --pull <policy>` overrides the per-service `pull_policy`.
		let requested = self
			.pull_policy_override
			.as_deref()
			.or(service.pull_policy.as_deref());
		let pull_policy = libpod_pull_policy(requested).unwrap_or_else(|| {
			warn!(
				"unknown pull policy '{}', defaulting to 'missing'",
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
	/// endpoint reports failures as in-band progress lines, not an HTTP error),
	/// and by the `up` image-prefetch stage to skip a redundant pull request
	/// for an image a `missing`-policy service already has cached.
	pub(in crate::engine) async fn image_present(&self, image: &str) -> bool {
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

	#[tokio::test]
	async fn pull_unknown_service_is_rejected() {
		// `pull bogus` must error on the unknown name instead of silently exiting 0.
		let file = crate::parse_str("services:\n  web:\n    image: a\n").unwrap();
		let e = crate::engine::Engine::new(
			crate::libpod::Client::new("/nonexistent.sock"),
			"proj".into(),
		);
		let err = e
			.pull_services(&file, &["nope".to_string()])
			.await
			.expect_err("unknown service must be rejected");
		assert!(
			matches!(err, crate::error::ComposeError::ServiceNotFound(_)),
			"unexpected error: {err:?}"
		);
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

	#[cfg(unix)]
	use crate::engine::fake_podman;

	/// #8: `pull_services_with_options` used to build one future per service,
	/// so two services sharing an image pulled it twice. They must now
	/// dedupe down to a single pull request, with both services still
	/// reported as successful.
	#[tokio::test]
	#[cfg(unix)]
	async fn pull_dedupes_a_shared_image_into_a_single_pull() {
		let fake = fake_podman::start(|method, target| {
			if method == "POST" && target.contains("/images/pull") {
				(200, String::new())
			} else if method == "GET" && target.contains("/images/") && target.contains("/json") {
				(200, "{}".to_string())
			} else {
				(404, r#"{"message":"not found"}"#.to_string())
			}
		});
		let e = crate::engine::Engine::new(fake.client(), "proj".into());
		let file =
			crate::parse_str("services:\n  a:\n    image: shared\n  b:\n    image: shared\n")
				.unwrap();

		e.pull_services(&file, &[])
			.await
			.expect("pulling two services that share an image must succeed");

		let seen = fake.requests.lock().unwrap();
		let pulls = seen
			.iter()
			.filter(|r| r.contains("/images/pull") && r.contains("reference=shared"))
			.count();
		assert_eq!(
			pulls, 1,
			"two services sharing one image must issue a single pull: {seen:?}"
		);
	}

	/// A shared image that fails to pull must still be reported for *every*
	/// service that names it — derived from the one shared outcome, not from
	/// a redundant pull per service. `ignore_failures` lets both warnings
	/// through instead of aborting on the first.
	#[tokio::test]
	#[cfg(unix)]
	async fn pull_failure_on_a_shared_image_is_still_only_pulled_once() {
		let fake = fake_podman::start(|method, target| {
			if method == "POST" && target.contains("/images/pull") {
				(500, r#"{"message":"registry unreachable"}"#.to_string())
			} else {
				(404, r#"{"message":"not found"}"#.to_string())
			}
		});
		let e = crate::engine::Engine::new(fake.client(), "proj".into());
		let file =
			crate::parse_str("services:\n  a:\n    image: shared\n  b:\n    image: shared\n")
				.unwrap();

		let opts = super::PullOptions {
			ignore_failures: true,
			include_deps: false,
		};
		e.pull_services_with_options(&file, &[], opts)
			.await
			.expect("ignore_failures must not error even though the shared pull failed");

		let seen = fake.requests.lock().unwrap();
		let pulls = seen
			.iter()
			.filter(|r| r.contains("/images/pull") && r.contains("reference=shared"))
			.count();
		assert_eq!(
			pulls, 1,
			"a failing shared image must still be pulled once, not once per service: {seen:?}"
		);
	}

	/// Without `ignore_failures`, a shared image that never lands must still
	/// abort the whole pull — the per-service error report is derived from
	/// the image's single shared outcome, so the failure is not silently
	/// dropped for services 2..N once service 1 already reported it.
	#[tokio::test]
	#[cfg(unix)]
	async fn pull_failure_on_a_shared_image_aborts_without_ignore_failures() {
		let fake = fake_podman::start(|method, target| {
			if method == "POST" && target.contains("/images/pull") {
				(500, r#"{"message":"registry unreachable"}"#.to_string())
			} else {
				(404, r#"{"message":"not found"}"#.to_string())
			}
		});
		let e = crate::engine::Engine::new(fake.client(), "proj".into());
		let file =
			crate::parse_str("services:\n  a:\n    image: shared\n  b:\n    image: shared\n")
				.unwrap();

		let err = e
			.pull_services(&file, &[])
			.await
			.expect_err("a shared image that fails to pull must abort the pull");
		assert!(
			matches!(err, crate::error::ComposeError::Build(ref msg) if msg.contains("shared")),
			"unexpected error: {err:?}"
		);
	}

	// Bounding the standalone pull's concurrency (`MAX_PULL_CONCURRENCY`) is
	// exercised structurally rather than by asserting a live in-flight count:
	// `bounded_join_all` runs every future through the same
	// `buffer_unordered(MAX_PULL_CONCURRENCY)` dispatcher the lifecycle
	// fan-out's `join_bounded` uses (see `parallel::tests::
	// join_bounded_preserves_input_order`), and a synchronous fake responder
	// cannot observe real concurrency without a multi-thread runtime and a
	// blocking rendezvous — exactly the flakiness the testing standard rules
	// out. The dedup tests above already pin the dispatch contract (every
	// unique image is attempted, exactly once).
}
