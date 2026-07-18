//! Shared healthcheck readiness for the concurrent `up` path.
//!
//! When several services in a dependency level declare `depends_on: <svc>:
//! {condition: service_healthy}`, they start concurrently and would each poll
//! that container's healthcheck — and every poll *runs* the check inside the
//! container, so a service N others wait on gets its healthcheck executed ~N×
//! per interval for the whole startup. [`Engine::build_readiness_map`] memoizes
//! one poller per container so the check runs once per interval regardless of
//! how many depend on it.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use futures_util::future::{FutureExt, Shared};

use crate::compose::types::{ComposeFile, ServiceCondition};
use crate::engine::Engine;
use crate::error::ComposeError;

use super::in_started_set;

/// A `wait_healthy` future shared across every dependent of one container, so a
/// service that N others wait on has its healthcheck polled by a single poller
/// rather than ~N× per interval. Lazy: the poll begins when the first dependent
/// awaits it. The error is `Arc`-wrapped because [`Shared`] needs a `Clone`
/// output and [`ComposeError`] is not `Clone`.
pub(super) type SharedReady<'a> =
	Shared<Pin<Box<dyn Future<Output = std::result::Result<(), Arc<ComposeError>>> + Send + 'a>>>;

impl Engine {
	/// Build one shared readiness future per container that any starting service
	/// waits on with `condition: service_healthy`.
	///
	/// The predicate mirrors the wait guard in `up_one_service`; a container it
	/// misses simply falls back to a direct wait there, so a mismatch degrades to
	/// the old per-dependent behaviour rather than a panic.
	pub(super) fn build_readiness_map<'a>(
		&'a self,
		file: &'a ComposeFile,
		enabled: &HashSet<String>,
		target_set: &Option<HashSet<String>>,
		start: bool,
	) -> HashMap<String, SharedReady<'a>> {
		let mut map: HashMap<String, SharedReady<'a>> = HashMap::new();
		// `create` (start = false) gates on nothing, so there are no waits to share.
		if !start {
			return map;
		}
		for (sname, service) in &file.services {
			// Only services this pass actually starts run their readiness waits.
			if let Some(set) = target_set {
				if !set.contains(sname) {
					continue;
				}
			}
			if !enabled.contains(sname) {
				continue;
			}
			for dep in service.depends_on.service_names() {
				if !matches!(
					service.depends_on.condition_for(&dep),
					ServiceCondition::ServiceHealthy
				) {
					continue;
				}
				if !in_started_set(target_set, &dep) {
					continue;
				}
				let Some(dep_service) = file.services.get(&dep) else {
					continue;
				};
				if !enabled.contains(&dep) {
					continue;
				}
				// A disabled healthcheck is treated as satisfied — never polled.
				if dep_service
					.healthcheck
					.as_ref()
					.is_some_and(|h| h.is_disabled())
				{
					continue;
				}
				let container = self.first_replica_name(&dep, dep_service);
				map.entry(container.clone()).or_insert_with(|| {
					let c = container.clone();
					async move {
						self.wait_healthy(&c, dep_service, None)
							.await
							.map_err(Arc::new)
					}
					.boxed()
					.shared()
				});
			}
		}
		map
	}
}

#[cfg(all(test, unix))]
mod tests {
	use std::collections::HashSet;

	use crate::engine::Engine;
	use crate::libpod::Client;

	fn engine(project: &str) -> Engine {
		// The map is built without any socket call (the shared futures are lazy),
		// so a client bound to a never-opened path is enough — no runtime needed.
		let client = Client::new("/tmp/podup-readiness-test.sock");
		Engine::with_base_dir(client, project.into(), std::env::temp_dir())
	}

	fn enabled_all(file: &crate::compose::types::ComposeFile) -> HashSet<String> {
		file.services.keys().cloned().collect()
	}

	#[test]
	fn shares_one_poller_per_service_healthy_container() {
		// web and api both wait on db with `service_healthy`; cache is waited on
		// with `service_started` (never polled). Exactly one shared entry — db's
		// container — must result, not one per dependent.
		let yaml = "\
services:
  db:
    image: x
    healthcheck:
      test: [\"CMD\", \"true\"]
  cache:
    image: x
  web:
    image: x
    depends_on:
      db:
        condition: service_healthy
      cache:
        condition: service_started
  api:
    image: x
    depends_on:
      db:
        condition: service_healthy
";
		let file = crate::compose::parse_str(yaml).unwrap();
		let e = engine("proj");
		let map = e.build_readiness_map(&file, &enabled_all(&file), &None, true);
		let keys: Vec<&String> = map.keys().collect();
		assert_eq!(map.len(), 1, "one shared poller expected, got {keys:?}");
		assert!(
			keys[0].contains("db"),
			"shared container should be db, got {keys:?}"
		);
	}

	#[test]
	fn create_only_shares_nothing() {
		// `create` (start = false) gates on no dependency, so nothing is shared.
		let yaml = "\
services:
  db:
    image: x
    healthcheck:
      test: [\"CMD\", \"true\"]
  web:
    image: x
    depends_on:
      db:
        condition: service_healthy
";
		let file = crate::compose::parse_str(yaml).unwrap();
		let e = engine("proj");
		assert!(e
			.build_readiness_map(&file, &enabled_all(&file), &None, false)
			.is_empty());
	}

	#[test]
	fn disabled_healthcheck_is_not_shared() {
		// A dependency whose healthcheck is disabled is treated as satisfied, so it
		// is never polled and must not get a shared poller.
		let yaml = "\
services:
  db:
    image: x
    healthcheck:
      disable: true
  web:
    image: x
    depends_on:
      db:
        condition: service_healthy
";
		let file = crate::compose::parse_str(yaml).unwrap();
		let e = engine("proj");
		assert!(
			e.build_readiness_map(&file, &enabled_all(&file), &None, true)
				.is_empty(),
			"a disabled healthcheck must not be shared or polled"
		);
	}
}
