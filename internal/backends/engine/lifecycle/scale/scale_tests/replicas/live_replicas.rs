use super::super::*;

/// libpod's `/containers/json` does not guarantee order; `logs` and every
/// other by-service lifecycle/query command must still see a scaled
/// service's replicas in the same ascending `-1, -2, -3` order the static
/// `replica_names_for` path always produces, even when the fake (like a
/// real libpod) hands them back shuffled. The bulk sorted helper
/// ([`Engine::live_project_replicas_sorted`]) is what now powers the
/// per-replica query paths (#1445); its per-bucket sort replaces the
/// per-service sort the old `live_replica_names` helper did.
#[tokio::test]
#[cfg(unix)]
async fn live_project_replicas_sorted_sorts_shuffled_replicas_ascending() {
	let containers = r#"[
		{"Names":["/proj-web-3"],"Labels":{"podup.service":"web"}},
		{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"}},
		{"Names":["/proj-web-2"],"Labels":{"podup.service":"web"}}
	]"#;
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, containers.to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let by_service = e
		.live_project_replicas_sorted()
		.await
		.expect("live_project_replicas_sorted should succeed");

	assert_eq!(
		by_service.get("web"),
		Some(&vec![
			"proj-web-1".to_string(),
			"proj-web-2".to_string(),
			"proj-web-3".to_string(),
		]),
		"the bucket must come back in ascending -1, -2, -3 order regardless of \
		 libpod's hand-back order"
	);
}

/// #1445: the bulk GET that the per-replica query paths now share must
/// cover a scaled service's full replica set, including replicas that are
/// stopped: `logs`/`port`/`exec` need to see the second replica even when
/// it is in `exited`, so the bulk request must NOT silently drop them the
/// way libpod's `runningOnly` default would.
#[tokio::test]
#[cfg(unix)]
async fn live_project_replicas_sorted_includes_stopped_replicas() {
	let containers = r#"[
		{"Names":["/proj-web-1"],"State":"running","Labels":{"podup.service":"web"}},
		{"Names":["/proj-web-2"],"State":"running","Labels":{"podup.service":"web"}},
		{"Names":["/proj-web-3"],"State":"exited","Labels":{"podup.service":"web"}}
	]"#;
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, containers.to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let by_service = e
		.live_project_replicas_sorted()
		.await
		.expect("live_project_replicas_sorted should succeed");

	// All three replicas, running and stopped, must be in the bucket.
	// A bug that silently filtered to running (libpod's `runningOnly`
	// default) would return just `proj-web-1` and `proj-web-2`, missing the
	// third replica entirely.
	assert_eq!(
		by_service.get("web"),
		Some(&vec![
			"proj-web-1".to_string(),
			"proj-web-2".to_string(),
			"proj-web-3".to_string(),
		]),
		"the stopped replica must not be filtered out; got {:?}",
		by_service.get("web")
	);
}

/// #1445: a service that exists in the compose file but has no live
/// container must NOT appear in the bulk map. The bulk helper does not see
/// the compose file; the per-replica query paths (`exec`/`cp`/`port`/
/// `logs`) layer the static-name fallback on top. Without that contract,
/// `logs` could not `docker compose logs`-style behave on a never-created
/// service: `port`/`exec`/`cp` would 404 immediately instead of letting
/// the caller see the predictable static name.
#[tokio::test]
#[cfg(unix)]
async fn live_project_replicas_sorted_omits_services_with_no_live_container() {
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/containers/json") {
			// Only `web` has any containers; `db` and `worker` are not yet
			// created.
			(
				200,
				r#"[{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"}}]"#.to_string(),
			)
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let by_service = e
		.live_project_replicas_sorted()
		.await
		.expect("live_project_replicas_sorted should succeed");

	assert_eq!(
		by_service.get("web"),
		Some(&vec!["proj-web-1".to_string()]),
		"a service with a live container must appear in the map"
	);
	assert!(
		!by_service.contains_key("db"),
		"a service with no live container must NOT appear (caller falls back): {by_service:?}"
	);
	assert!(
		!by_service.contains_key("worker"),
		"a service with no live container must NOT appear (caller falls back): {by_service:?}"
	);
}

/// #1445: the per-replica query paths used to fan out one container-list
/// round-trip per service: 40 services meant 40 GETs for a single
/// `podup logs`. The bulk helper is the shared single GET. With the fake
/// pinned to answer only `/containers/json`, a single resolution call must
/// still satisfy the per-service query helpers without issuing a follow-up
/// GET. A regression that re-introduced per-service calls would make
/// `fake.requests` grow by one row for each lookup, so the assertion
/// below is the discriminator: counting requests is the test that fails if
/// the bulk path is bypassed.
#[tokio::test]
#[cfg(unix)]
async fn logs_resolves_replicas_in_one_bulk_get_not_one_per_service() {
	// Three services; `db` is scaled to 3, `web` to 2, `worker` to 1, all
	// running, so the bulk GET is the only fetch needed for `logs` to
	// find 6 targets across 3 services.
	let body = r#"[
		{"Names":["/proj-db-1"],"Labels":{"podup.service":"db"}},
		{"Names":["/proj-db-2"],"Labels":{"podup.service":"db"}},
		{"Names":["/proj-db-3"],"Labels":{"podup.service":"db"}},
		{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"}},
		{"Names":["/proj-web-2"],"Labels":{"podup.service":"web"}},
		{"Names":["/proj-worker-1"],"Labels":{"podup.service":"worker"}}
	]"#;
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, body.to_string())
		} else if method == "GET" && target.contains("/logs") {
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let mut file = crate::compose::types::ComposeFile::default();
	file.services
		.insert("db".into(), crate::compose::types::Service::default());
	file.services
		.insert("web".into(), crate::compose::types::Service::default());
	file.services
		.insert("worker".into(), crate::compose::types::Service::default());

	// Drive `logs` once across all 3 services; the resolution path is the
	// one under test, not the streaming.
	let _ = e
		.logs_with_options(&file, &[], crate::engine::query::LogsOptions::default())
		.await;

	let seen = fake.requests.lock().unwrap();
	let bulk_gets = seen
		.iter()
		.filter(|r| r.starts_with("GET") && r.contains("/containers/json"))
		.count();
	assert_eq!(
		bulk_gets, 1,
		"logs must issue exactly one bulk GET for the whole project, \
		 not one per service. seen = {seen:?}"
	);
}
