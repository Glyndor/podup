//! A level that fails to stop does not spare the levels below it.

use crate::engine::fake_podman;
use crate::engine::Engine;

const LIVE: &str = r#"[
	{"Names":["/proj-web-1"],"State":"running","Labels":{"podup.service":"web"}},
	{"Names":["/proj-db-1"],"State":"running","Labels":{"podup.service":"db"}}
]"#;

#[tokio::test]
async fn the_dependency_is_still_stopped_when_its_dependent_cannot_be() {
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, LIVE.to_string())
		} else if method == "POST" && target.contains("proj-web-1/stop") {
			(500, r#"{"message":"stop refused"}"#.to_string())
		} else {
			(204, String::new())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	let file = crate::parse_str(
		"services:\n  db:\n    image: x\n  web:\n    image: x\n    depends_on: [db]\n",
	)
	.unwrap();
	let error = engine
		.stop(&file, &[])
		.await
		.expect_err("web could not be stopped");
	assert!(error.to_string().contains("stop refused"), "{error}");
	let seen = fake.requests.lock().unwrap().clone();
	assert!(
		seen.iter().any(|r| r.contains("proj-db-1/stop")),
		"db was left running: {seen:?}"
	);
}
