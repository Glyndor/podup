//! A one-off container is not a replica of its service.
//!
//! `podup run` creates a container that carries the service label of the
//! service it ran, so the bulk project listing counts it among that service's
//! containers. It is not one of the service's replicas: it exists to be run
//! once, and it is labelled `podup.oneoff=true` precisely so the two can be
//! told apart. A `start` or a `restart` that acts on it runs the task a
//! second time, which is what the label exists to prevent.

use crate::engine::fake_podman::{self, FakePodman};
use crate::engine::Engine;

/// The project's containers: one replica of `web`, and the one-off container
/// a previous `podup run web` left behind.
const LIVE: &str = r#"[
	{"Names":["/proj-web-1"],"State":"exited","Labels":{"podup.service":"web"}},
	{"Names":["/proj-web-run-4242"],"State":"exited","Labels":{"podup.service":"web","podup.oneoff":"true"}}
]"#;

/// A fake that answers the project listing and accepts every state change.
fn engine() -> (FakePodman, Engine) {
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, LIVE.to_string())
		} else {
			(200, String::new())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	(fake, engine)
}

fn file() -> crate::compose::types::ComposeFile {
	crate::parse_str("services:\n  web:\n    image: alpine\n").expect("the fixture parses")
}

/// The requests that act on a container, with the project listing left out.
fn actions(fake: &FakePodman) -> Vec<String> {
	fake.requests
		.lock()
		.unwrap()
		.iter()
		.filter(|r| r.contains("/containers/") && !r.contains("/containers/json"))
		.cloned()
		.collect()
}

/// `start` starts the service's replicas and nothing else: the one-off
/// container is not a replica, so it stays where it is.
#[tokio::test]
async fn start_leaves_a_one_off_container_alone() {
	let (fake, engine) = engine();

	engine
		.start(&file(), &["web".to_string()])
		.await
		.expect("start succeeds");

	let actions = actions(&fake);
	assert!(
		actions.iter().any(|a| a.contains("proj-web-1/start")),
		"the service's replica is started: {actions:?}"
	);
	assert!(
		!actions.iter().any(|a| a.contains("proj-web-run-4242")),
		"the one-off container must not be started again: {actions:?}"
	);
}

/// `restart` restarts the service's replicas and nothing else: the one-off
/// container is not a replica, so it stays where it is.
#[tokio::test]
async fn restart_leaves_a_one_off_container_alone() {
	let (fake, engine) = engine();

	engine
		.restart_with_options(&file(), &["web".to_string()], false)
		.await
		.expect("restart succeeds");

	let actions = actions(&fake);
	assert!(
		actions.iter().any(|a| a.contains("proj-web-1/restart")),
		"the service's replica is restarted: {actions:?}"
	);
	assert!(
		!actions.iter().any(|a| a.contains("proj-web-run-4242")),
		"the one-off container must not be restarted again: {actions:?}"
	);
}
