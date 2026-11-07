//! `run` waits for the removal of an init container that its own step could not
//! remove: the guard finishes it in the background, and the process ends right
//! after the error.

use super::RunOptions;
use crate::engine::fake_podman;
use crate::engine::Engine;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_removal_a_failed_step_leaves_to_the_guard_finishes_before_run_returns() {
	let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
	let removed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
	let (count, flag) = (calls.clone(), removed.clone());
	let fake = fake_podman::start(move |method, target| {
		if method == "DELETE" && target.contains("pre-start") {
			// The step's own removal is refused; the guard's second try is slow.
			if count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
				(500, r#"{"message":"remove refused"}"#.to_string())
			} else {
				std::thread::sleep(std::time::Duration::from_millis(600));
				flag.store(true, std::sync::atomic::Ordering::SeqCst);
				(204, String::new())
			}
		} else if method == "POST" && target.ends_with("/containers/create") {
			(200, r#"{"Id":"init-1"}"#.to_string())
		} else if method == "POST" && target.ends_with("/start") {
			(204, String::new())
		} else if method == "POST" && target.contains("/wait") {
			(200, "0".to_string())
		} else if method == "GET" && target.contains("/images/") {
			(200, r#"{"Id":"sha256:0123"}"#.to_string())
		} else if method == "POST" && target.contains("/images/pull") {
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	let file = crate::parse_str(
		"services:\n  web:\n    image: base\n    pre_start:\n      - command: init\n",
	)
	.unwrap();
	let options = RunOptions {
		detach: true,
		..RunOptions::default()
	};
	let outcome = engine.run(&file, "web", options).await;
	assert!(outcome.is_err(), "the step's removal was refused");
	assert!(
		removed.load(std::sync::atomic::Ordering::SeqCst),
		"run returned while the guard's removal was still in flight"
	);
}
