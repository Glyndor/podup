//! A dependency declared `required: false` that fails to start must not stop
//! `up` or the services that asked for it only optionally.

use crate::engine::fake_podman;
use crate::engine::lifecycle::Engine;

const OPTIONAL: &str = "services:\n  metrics:\n    image: img-metrics\n  web:\n    image: img-web\n    depends_on:\n      metrics:\n        condition: service_started\n        required: false\n";
const REQUIRED: &str = "services:\n  metrics:\n    image: img-metrics\n  web:\n    image: img-web\n    depends_on:\n      metrics:\n        condition: service_started\n";

fn engine(fake: &fake_podman::FakePodman) -> Engine {
	Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir())
}

/// Every call succeeds except the start of `proj-metrics-1`, which fails the
/// way a container does when its port is taken.
fn metrics_cannot_start() -> fake_podman::FakePodman {
	fake_podman::start(|method, target| {
		if method == "POST" && target.contains("/proj-metrics-1/start") {
			(500, r#"{"message":"address already in use"}"#.to_string())
		} else if method == "GET" && target.contains("/containers/json") {
			(200, "[]".to_string())
		} else if method == "POST" && target.contains("/containers/create") {
			(200, r#"{"Id":"abc"}"#.to_string())
		} else if method == "POST" && (target.contains("/start") || target.contains("/images/pull"))
		{
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	})
}

#[tokio::test]
async fn an_optional_dependency_that_fails_does_not_stop_up_or_its_dependent() {
	let fake = metrics_cannot_start();
	let file = crate::parse_str(OPTIONAL).unwrap();
	engine(&fake)
		.up_with_options(&file, false, &[], &[], false, false, false, false)
		.await
		.expect("an optional dependency failing is a warning, not an error");
	let seen = fake.requests.lock().unwrap();
	assert!(
		seen.iter().any(|r| r.contains("/proj-web-1/start")),
		"the dependent must still start: {seen:?}"
	);
}

#[tokio::test]
async fn a_required_dependency_that_fails_still_stops_up() {
	let fake = metrics_cannot_start();
	let file = crate::parse_str(REQUIRED).unwrap();
	let err = engine(&fake)
		.up_with_options(&file, false, &[], &[], false, false, false, false)
		.await
		.expect_err("a required dependency failing must fail up");
	assert!(err.to_string().contains("address already in use"), "{err}");
}
