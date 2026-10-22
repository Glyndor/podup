use super::*;

/// `ps` hides the infra container (`IsInfra: true` on the list response).
#[tokio::test]
#[cfg(unix)]
async fn ps_hides_the_infra_container() {
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/containers/json") {
			// Two containers: one regular, one infra.
			(
				200,
				r#"[
					{"Id":"aaa","Names":["/proj-web-1"],"Image":"nginx","ImageID":"sha256:web","Status":"","State":"running","Ports":[],"Labels":{"podup.project":"proj","podup.service":"web"},"Created":"2026-01-01T00:00:00Z","IsInfra":false},
					{"Id":"bbb","Names":["/proj-infra"],"Image":"k8s.gcr.io/pause:3.5","ImageID":"sha256:pause","Status":"","State":"running","Ports":[],"Labels":{"podup.project":"proj"},"Created":"2026-01-01T00:00:00Z","IsInfra":true}
				]"#
					.to_string(),
			)
		} else {
			(404, r#"{"message":"unexpected"}"#.to_string())
		}
	});
	let engine = engine_for(&fake, "proj");
	let file = parse_str("services:\n  web:\n    image: nginx\n").unwrap();
	// Use the test-only row helper so the test can introspect the rows
	// directly without going through stdout.
	let json_rows = crate::engine::query::ps_rows_for_test(&engine, &file)
		.await
		.expect("ps must succeed");
	let names: Vec<&str> = json_rows
		.iter()
		.map(|r| r["Name"].as_str().unwrap_or(""))
		.collect();
	assert!(
		names.contains(&"proj-web-1"),
		"regular container must be listed: {names:?}"
	);
	assert!(
		!names.iter().any(|n| n.contains("infra")),
		"infra container must be hidden from ps; got: {names:?}"
	);
}
