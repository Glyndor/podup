use super::*;

/// #1363: the bulk project listing must include every project's
/// containers, not just the running ones. With `all=true` libpod drops the
/// `runningOnly` default and a 100-service project with 5 stopped replicas
/// returns all 100 names; the gotcha caught during validation was a bare
/// `GET /containers/json` (no `all=true`, no `status=...`) silently
/// filtering the stopped ones out, which would have made `start`/`restart`
/// act on a strict subset of the project's containers and leave the rest
/// untouched. Asserts both the request shape (carries `all=true`, no
/// `status=` workaround) and the returned map (every service present,
/// 100 names total).
#[tokio::test]
#[cfg(unix)]
async fn live_project_replicas_returns_every_project_container_including_stopped() {
	// 100 services; 95 running + 5 stopped, one container each.
	let entries: Vec<String> = (0..100)
		.map(|i| {
			let svc = format!("svc{i:03}");
			let state = if i % 20 == 0 { "exited" } else { "running" };
			format!(
				r#"{{"Names":["/proj-{svc}-1"],"State":"{state}","Labels":{{"podup.service":"{svc}"}}}}"#
			)
		})
		.collect();
	let body = format!("[{}]", entries.join(","));
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, body.clone())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let by_service = e
		.live_project_replicas()
		.await
		.expect("live_project_replicas should succeed");

	assert_eq!(
		by_service.len(),
		100,
		"every project service must be in the map, got {} entries",
		by_service.len()
	);
	let total_names: usize = by_service.values().map(Vec::len).sum();
	assert_eq!(
		total_names, 100,
		"every project container must be returned (5 stopped included)"
	);
	// The 5 stopped services (svc000, svc020, …, svc080) must each be
	// present with their replica name, i.e. `all=true` actually carried
	// past the default filter, not just "runningOnly expanded to one".
	for i in (0..100).step_by(20) {
		let svc = format!("svc{i:03}");
		let names = by_service
			.get(&svc)
			.unwrap_or_else(|| panic!("missing {svc}"));
		assert_eq!(names, &vec![format!("proj-{svc}-1")], "{svc}");
	}

	// The request path must include `all=true`: libpod's container-list
	// defaults to `runningOnly` when no `status` filter is supplied, so
	// the bulk GET that powers the per-service lifecycle commands would
	// silently drop the very containers those commands need to act on.
	let seen = fake.requests.lock().unwrap();
	let container_list = seen
		.iter()
		.find(|r| r.contains("GET") && r.contains("/containers/json"))
		.expect("exactly one bulk GET to /containers/json");
	assert!(
		container_list.contains("all=true"),
		"the bulk GET must pass all=true, got {container_list:?}"
	);
	// And no `status=` filter sneaked in that would have achieved the
	// same effect; we want the all-inclusive default, not a workaround.
	assert!(
		!container_list.contains("status="),
		"the bulk GET must rely on all=true alone, not a status filter: {container_list:?}"
	);
}
