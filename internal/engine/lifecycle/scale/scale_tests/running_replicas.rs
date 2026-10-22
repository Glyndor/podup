use super::*;

/// #1250 / #1742: `top` aborts on a project with a stopped service because
/// `/top` answers a non-running container with an HTTP 500. The bulk
/// `live_project_running_replicas_sorted` helper (the one `top` now reads
/// from) must drop non-running replicas before the call, and the survivors
/// must keep the ascending `-1, -2, -3` order every other by-service
/// command produces. Both come back from a single `/containers/json` round
/// trip: the same listing is shared across all services.
#[tokio::test]
#[cfg(unix)]
async fn live_project_running_replicas_sorted_drops_non_running_and_keeps_ascending_order() {
	let containers = r#"[
		{"Names":["/proj-web-3"],"State":"running","Labels":{"podup.service":"web"}},
		{"Names":["/proj-web-4"],"State":"created","Labels":{"podup.service":"web"}},
		{"Names":["/proj-web-1"],"State":"running","Labels":{"podup.service":"web"}},
		{"Names":["/proj-web-5"],"State":"paused","Labels":{"podup.service":"web"}},
		{"Names":["/proj-web-2"],"State":"exited","Labels":{"podup.service":"web"}}
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
		.live_project_running_replicas_sorted()
		.await
		.expect("live_project_running_replicas_sorted should succeed");

	assert_eq!(
		by_service.get("web"),
		Some(&vec!["proj-web-1".to_string(), "proj-web-3".to_string()]),
		"only the running replicas, in ascending order"
	);
}

/// The sibling half of the rule above: a service that exists in the compose
/// file but was never created has nothing running, so `top` must render
/// nothing for it rather than fall back to a statically-derived name and
/// then have to swallow the 404 that name earns.
#[tokio::test]
#[cfg(unix)]
async fn live_project_running_replicas_sorted_yields_no_names_for_a_never_created_service() {
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, "[]".to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let by_service = e
		.live_project_running_replicas_sorted()
		.await
		.expect("live_project_running_replicas_sorted should succeed");

	assert!(
		by_service
			.get("web")
			.map(Vec::as_slice)
			.unwrap_or(&[])
			.is_empty(),
		"a never-created service yields no names, got {by_service:?}"
	);
}
