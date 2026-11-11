//! A cancelled init operation hands the removal of its container to a
//! background task, and `up` waits for it before it returns.

use crate::engine::fake_podman;

/// With the removal still pending when the drain returns, the process would
/// exit right after the error that cancelled the operation and take it along.
#[tokio::test]
async fn draining_waits_for_the_removal_of_a_cancelled_init_container() {
	let fake = fake_podman::start(|_, _| (204, String::new()));
	let client = fake.client();
	drop(client.init_cleanup("/libpod/containers/init-1?force=true&volumes=true".into()));
	client.drain_init_cleanups().await;
	let seen = fake.requests.lock().unwrap().clone();
	assert!(
		seen.iter()
			.any(|r| r.starts_with("DELETE") && r.contains("init-1")),
		"the removal had not reached the daemon when the drain returned: {seen:?}"
	);
}

#[tokio::test]
async fn a_removal_that_was_disarmed_leaves_nothing_to_wait_for() {
	let fake = fake_podman::start(|_, _| (204, String::new()));
	let client = fake.client();
	let mut guard = client.init_cleanup("/libpod/containers/init-2".into());
	guard.disarm();
	drop(guard);
	client.drain_init_cleanups().await;
	assert!(fake.requests.lock().unwrap().is_empty());
}

/// `up` itself waits: a removal scheduled by a cancelled service is finished
/// before `up` hands back the error of the sibling that cancelled it. The
/// daemon answers the removal slowly while everything else fails at once, so
/// only waiting for it can make it complete first.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn up_finishes_the_pending_removals_before_it_returns_its_error() {
	let removed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
	let flag = removed.clone();
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, "[]".to_string())
		} else if method == "DELETE" && target.contains("init-3") {
			std::thread::sleep(std::time::Duration::from_millis(600));
			flag.store(true, std::sync::atomic::Ordering::SeqCst);
			(204, String::new())
		} else if method == "DELETE" {
			(204, String::new())
		} else {
			(500, r#"{"message":"no"}"#.to_string())
		}
	});
	let client = fake.client();
	drop(client.init_cleanup("/libpod/containers/init-3?force=true&volumes=true".into()));
	let engine = crate::engine::Engine::with_base_dir(client, "proj".into(), std::env::temp_dir());
	let file = crate::parse_str("services:\n  web:\n    image: img\n").unwrap();
	let outcome = engine
		.up_with_options(&file, false, &[], &[], false, false, false, false)
		.await;
	assert!(outcome.is_err(), "the fake refuses everything else");
	assert!(
		removed.load(std::sync::atomic::Ordering::SeqCst),
		"up returned while the removal was still in flight"
	);
}
