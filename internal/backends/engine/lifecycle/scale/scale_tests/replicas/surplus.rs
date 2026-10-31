use super::super::*;

/// #598: a `scale`/`up --scale` down-sizing that can't remove a surplus
/// replica (e.g. an active exec session) must not exit 0 with it left
/// running, but a sibling replica that removes cleanly must still be
/// reclaimed.
#[tokio::test]
#[cfg(unix)]
async fn remove_surplus_replicas_propagates_a_real_rm_failure_after_completing_the_rest() {
	let live = r#"[{"Names":["/proj-web-1"]},{"Names":["/proj-web-2"]}]"#;
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, live.to_string())
		} else if (method == "POST" && target.contains("/stop"))
			|| (method == "DELETE" && target.contains("/proj-web-1?force=true"))
		{
			(200, String::new())
		} else if method == "DELETE" && target.contains("/proj-web-2?force=true") {
			(500, r#"{"message":"device or resource busy"}"#.to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	// Build the snapshot `run_up` would have handed `remove_surplus_replicas`
	// in production: every container in the bulk fetch carries the
	// `podup.service` label that the per-service filter used to look up
	// (`#1747`). `target = 0` makes every name a surplus replica.
	let mut existing: std::collections::HashMap<
		String,
		crate::engine::lifecycle::ExistingContainer,
	> = std::collections::HashMap::new();
	for raw in ["proj-web-1", "proj-web-2"] {
		existing.insert(
			raw.to_string(),
			crate::engine::lifecycle::ExistingContainer {
				config_hash: None,
				image_id: String::new(),
				service: Some("web".to_string()),
				oneoff: false,
			},
		);
	}
	let err = e
		.remove_surplus_replicas(
			"web",
			&crate::compose::types::Service::default(),
			0,
			&existing,
		)
		.await
		.expect_err("a real surplus-removal failure must propagate");
	assert!(
		matches!(err, ComposeError::Podman(ref pe) if pe.is_status(500)),
		"got {err:?}"
	);

	let seen = fake.requests.lock().unwrap();
	assert!(
		seen.iter()
			.any(|r| r.contains("DELETE") && r.contains("/proj-web-1?force=true")),
		"expected proj-web-1 to still be removed despite proj-web-2 failing: {seen:?}"
	);
	// The fix takes the snapshot as a parameter; the per-service fetch that
	// used to live inside `remove_surplus_replicas` is gone (#1747), so
	// `/containers/json` should not appear in the recorded requests at all.
	assert!(
		seen.iter()
			.all(|r| !r.contains("GET") || !r.contains("/containers/json")),
		"remove_surplus_replicas must not fetch /containers/json: {seen:?}"
	);
}

/// Surplus replicas that are already gone (404 on removal) stay an
/// idempotent no-op: a re-run of `scale` down must still exit 0.
#[tokio::test]
#[cfg(unix)]
async fn remove_surplus_replicas_tolerates_already_gone() {
	// Build the snapshot directly: the fix takes the bulk project's
	// container list as a parameter rather than refetching (`#1747`).
	let mut existing: std::collections::HashMap<
		String,
		crate::engine::lifecycle::ExistingContainer,
	> = std::collections::HashMap::new();
	existing.insert(
		"proj-web-1".to_string(),
		crate::engine::lifecycle::ExistingContainer {
			config_hash: None,
			image_id: String::new(),
			service: Some("web".to_string()),
			oneoff: false,
		},
	);
	let fake = fake_podman::start(move |method, target| {
		// `remove_surplus_replicas` no longer fetches `/containers/json`,
		// so any GET there is a regression. The stop/remove pair is
		// answered 404 so the function sees the already-gone case.
		if method == "POST" && target.contains("/stop")
			|| method == "DELETE" && target.contains("/proj-web-1?force=true")
		{
			(404, r#"{"message":"no such container"}"#.to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");
	e.remove_surplus_replicas(
		"web",
		&crate::compose::types::Service::default(),
		0,
		&existing,
	)
	.await
	.expect("an already-gone surplus replica must still exit 0");

	let seen = fake.requests.lock().unwrap();
	assert!(
		seen.iter()
			.all(|r| !r.contains("GET") || !r.contains("/containers/json")),
		"remove_surplus_replicas must not issue the project list query: {seen:?}"
	);
}
