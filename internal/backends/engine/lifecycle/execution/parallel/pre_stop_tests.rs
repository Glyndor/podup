//! `stop` and `restart` run the `pre_stop` hooks of a service, as `down` does.

use crate::engine::fake_podman;
use crate::engine::Engine;

const LIVE: &str = r#"[
	{"Id":"aaaa1111","Names":["/proj-web-1"],"State":"running","Labels":{"podup.service":"web"}},
	{"Id":"bbbb2222","Names":["/proj-web-2"],"State":"running","Labels":{"podup.service":"web"}}
]"#;

fn engine() -> (fake_podman::FakePodman, Engine) {
	// Anything about exec is refused: a hook that cannot run must not stop the stop.
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, LIVE.to_string())
		} else if target.contains("/exec") {
			(404, r#"{"message":"no exec"}"#.to_string())
		} else {
			(204, String::new())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	(fake, engine)
}

fn file(hooks: &str) -> crate::compose::types::ComposeFile {
	crate::parse_str(&format!(
		"services:\n  web:\n    image: x\n    scale: 2\n{hooks}"
	))
	.unwrap()
}

const HOOK: &str = "    pre_stop:\n      - command: echo bye\n";

/// Index of the first request containing `needle`.
fn at(requests: &[String], needle: &str) -> Option<usize> {
	requests.iter().position(|r| r.contains(needle))
}

#[tokio::test]
async fn stop_runs_the_hook_in_every_replica_before_its_stop() {
	let (fake, engine) = engine();
	engine.stop(&file(HOOK), &[]).await.unwrap();
	let seen = fake.requests.lock().unwrap().clone();
	for replica in ["proj-web-1", "proj-web-2"] {
		let exec = at(&seen, &format!("{replica}/exec")).unwrap_or(usize::MAX);
		let stop = at(&seen, &format!("{replica}/stop")).expect("the replica is stopped");
		assert!(exec < stop, "{replica}: hook first, then stop: {seen:?}");
	}
}

#[tokio::test]
async fn restart_runs_the_hook_before_the_restart() {
	let (fake, engine) = engine();
	engine
		.restart_with_options(&file(HOOK), &[], false)
		.await
		.unwrap();
	let seen = fake.requests.lock().unwrap().clone();
	let exec = at(&seen, "proj-web-1/exec").unwrap_or(usize::MAX);
	let restart = at(&seen, "proj-web-1/restart").expect("the replica is restarted");
	assert!(exec < restart, "{seen:?}");
}

#[tokio::test]
async fn the_final_stop_of_a_snapshot_runs_the_hook_in_the_container_of_the_snapshot() {
	let (fake, engine) = engine();
	let file = file(HOOK);
	let snapshot = engine.snapshot_containers().await.unwrap();
	engine.stop_snapshot(&file, &snapshot).await.unwrap();
	let seen = fake.requests.lock().unwrap().clone();
	assert!(
		at(&seen, "aaaa1111/exec").is_some(),
		"by id, not by name: {seen:?}"
	);
	assert!(at(&seen, "proj-web-1/exec").is_none(), "{seen:?}");
}

#[tokio::test]
async fn a_service_without_hooks_gets_no_exec() {
	let (fake, engine) = engine();
	engine.stop(&file(""), &[]).await.unwrap();
	let seen = fake.requests.lock().unwrap().clone();
	assert!(at(&seen, "/exec").is_none(), "{seen:?}");
}
