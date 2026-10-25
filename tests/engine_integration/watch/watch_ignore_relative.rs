//! Live test: a watch rule's `ignore:` is matched against the path
//! relative to the rule's `path`, not the project root.
//!
//! Regression for the case where a rule `path: ./src` with
//! `ignore: ["*.tmp"]` failed to ignore `src/a.tmp` because the previous
//! matcher was project-relative and looked for `*.tmp` against the full
//! `src/a.tmp` path; the rule's intended `path: ./src` meant the matcher
//! should have been looking at `a.tmp` (the path relative to `./src`),
//! and `a.tmp` does match `*.tmp`. With the new matcher the spec
//! correctly identifies `*.tmp` against the rule-relative path and the
//! `a.tmp` write does not reach the container, while a sibling `b.txt`
//! write does. Both writes happen on the host; the assertion reads the
//! container back out to confirm what landed and what did not.
//!
//! `PODUP_REQUIRE_PODMAN=1` makes a missing Podman fail the suite instead
//! of silently passing, since the only observation of the ignore decision
//! is reading the container back. The drop guard tears the project down
//! so a panic cannot leave containers behind for the next test in the
//! suite to inherit.
#![cfg(feature = "test-helpers")]

use std::time::Duration;

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_ignore_pattern_uses_rule_relative_path() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let src = dir.path().join("src");
	fs::create_dir(&src).unwrap();

	let proj = proj("wirrp");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    develop:\n      watch:\n        - path: ./src\n          action: sync\n          target: /tmp\n          ignore: [\"*.tmp\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();

	let client2 = podup::podman::connect_from_env()
		.or_else(|_| podup::podman::connect(None))
		.unwrap();
	let engine2 = Engine::with_base_dir(client2, proj.clone(), dir.path().to_path_buf());
	let file2 = file.clone();
	let mut handle = tokio::spawn(async move { engine2.watch(&file2).await });

	// Give the watcher a moment to register before changing the files,
	// then poll for the two effects rather than assuming a fixed sync
	// duration. Routed through `poll_with_watch` so a watch task that
	// died panics with the watch error instead of blaming the ignore.
	tokio::time::sleep(Duration::from_secs(2)).await;
	fs::write(src.join("a.tmp"), b"ignored-by-pattern").unwrap();
	fs::write(src.join("b.txt"), b"kept-by-default").unwrap();

	let cname = format!("{proj}-web-1");
	let b_arrived = poll_with_watch(&mut handle, Duration::from_secs(60), || async {
		if let Ok(out) = engine
			.test_exec_capture(&cname, vec!["cat".into(), "/tmp/b.txt".into()])
			.await
		{
			out.contains("kept-by-default")
		} else {
			false
		}
	})
	.await;

	// Confirm the `a.tmp` write did NOT make it across. The wait is short
	// on purpose: if the new matcher is broken and a.tmp reaches the
	// container the assertion fails quickly, instead of a fixed sleep
	// making the test pass on a slow machine by accident.
	let a_reached = poll_with_watch(&mut handle, Duration::from_secs(5), || async {
		if let Ok(out) = engine
			.test_exec_capture(&cname, vec!["cat".into(), "/tmp/a.tmp".into()])
			.await
		{
			out.contains("ignored-by-pattern")
		} else {
			false
		}
	})
	.await;

	handle.abort();
	engine.down(&file).await.unwrap();
	assert!(
		b_arrived,
		"the watcher ignored b.txt; rule-relative sync must deliver b.txt to the container"
	);
	assert!(
		!a_reached,
		"the watcher synced a.tmp; rule-relative `ignore: [\"*.tmp\"]` against `./src/a.tmp` must skip the file"
	);
}
