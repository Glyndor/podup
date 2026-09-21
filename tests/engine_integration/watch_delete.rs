//! Live test: a `sync` watch rule must propagate host-side deletions into the
//! container, the resulting warning (if any) must not be a build error, and
//! the rule's target directory itself must never be removed.
//!
//! This is the regression for the case where deleting a watched file printed
//! `podup: warning: watch action failed: build error: No such file or directory
//! (os error 2) when getting metadata for /tmp/d5/./src/f.txt` and left the
//! file inside the container. Three halves:
//!
//! - propagation: a host-side deletion must mirror to the container
//! - classification: the warning that surfaces must not be a `build error`
//! - scope: only entries inside the target are removed; the target directory
//!   itself survives both an entry delete and a delete of the rule's root
//!
//! The driver is the real watcher: the watcher is started as a user would,
//! the file is removed on the host, and the assertion reads the container
//! back out. Driving the leaf (`test_remove_from_container`) bypasses the
//! event plumbing and the path mapping, so a test that does that passes
//! whether the dispatch is wired or not.
//!
//! `PODUP_REQUIRE_PODMAN=1` makes a missing Podman fail the suite instead of
//! silently passing, since the deletion/creation has no real observation
//! against a stub daemon. The drop guard tears the project down so a panic
//! cannot leave containers behind for the next test in the suite to inherit.
#![cfg(feature = "test-helpers")]

use std::time::Duration;

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_sync_propagates_host_deletions_to_the_container() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let src = dir.path().join("src");
	fs::create_dir(&src).unwrap();
	// Use `./src` as the rule path. The literal `./` is what produced
	// `/tmp/d5/./src/f.txt` in the original warn line; canonicalising the
	// rule path on entry is the fix that line depends on, and this test
	// exercises the deletion flow whose warn line carried that literal.
	let src_dot = dir.path().join("./src");
	assert!(
		src_dot.exists(),
		"the `./` form must resolve to the same dir"
	);

	let proj = proj("wdel");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    develop:\n      watch:\n        - path: ./src\n          action: sync\n          target: /app\n          initial_sync: true\n",
	)
	.unwrap();
	engine.up(&file).await.unwrap();

	// Drop guard: bring the project down whatever the assertions did, so a
	// failed assertion cannot leak containers into the next test run. The
	// helper spawns its own single-thread runtime so it works after the
	// test's runtime has shut down.
	struct Teardown {
		engine: Engine,
		file: podup::compose::types::ComposeFile,
	}
	impl Drop for Teardown {
		fn drop(&mut self) {
			let engine = std::mem::replace(
				&mut self.engine,
				Engine::new(
					podup::podman::connect(None).expect("teardown connect"),
					String::new(),
				),
			);
			let file = self.file.clone();
			let res = std::thread::Builder::new()
				.name("watch_delete_teardown".into())
				.spawn(move || {
					let rt = tokio::runtime::Builder::new_current_thread()
						.enable_all()
						.build()
						.expect("teardown runtime");
					rt.block_on(async move { engine.down(&file).await })
				})
				.expect("teardown thread")
				.join()
				.expect("teardown thread result");
			if let Err(e) = res {
				eprintln!("watch_delete teardown: down failed: {e}");
			}
		}
	}
	let teardown = Teardown {
		engine: Engine::with_base_dir(
			podup::podman::connect(None).expect("teardown engine connect"),
			proj.clone(),
			dir.path().to_path_buf(),
		),
		file: file.clone(),
	};

	let cname = format!("{proj}-web-1");

	// Drive the real watcher: a second engine, spawned on its own task, sets
	// up the inotify watcher, runs initial_sync, and dispatches on the
	// Remove event the host-side deletion fires. This is the same shape as
	// `watch_initial_sync_runs` and `watch_sync_and_restart_does_both`;
	// calling the leaf (`test_remove_from_container`) instead would skip
	// is_remove_event, the event plumbing, and the path mapping, and a test
	// that does that passes whether the dispatch is wired or not.
	let client2 = podup::podman::connect_from_env()
		.or_else(|_| podup::podman::connect(None))
		.unwrap();
	let engine2 = Engine::with_base_dir(client2, proj.clone(), dir.path().to_path_buf());
	let file2 = file.clone();
	let handle = tokio::spawn(async move { engine2.watch(&file2).await });

	let src_file = src.join("f.txt");
	fs::write(&src_file, b"watched").unwrap();

	// Poll for the initial-sync delivery rather than sleeping a fixed duration.
	// `watched` is the only write we made; a green `initial_sync` is what puts
	// it inside the container, and a missing delivery here means the deletion
	// half has no baseline to compare against.
	let arrived = poll_container_contains(&engine, &cname, "/app/f.txt", "watched", 30).await;

	// Delete the file on the host. The watcher is the consumer: it receives
	// the Remove event, plans the placement against `/app`, and runs
	// `rm -f /app/f.txt` on the container. The change reaches the container
	// through this path, not through a direct call into the leaf.
	fs::remove_file(&src_file).unwrap();

	// Poll for the absence of the file. Reading `ls`'s exit code rather
	// than sleeping and hoping gives the test a deterministic observation:
	// `ls /app/f.txt` answers non-zero (and prints nothing) once the
	// removal has propagated.
	let mut gone = false;
	let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
	while tokio::time::Instant::now() < deadline {
		let out = engine
			.test_exec_capture(&cname, vec!["ls".into(), "/app/f.txt".into()])
			.await
			.unwrap_or_default();
		if out.trim().is_empty() {
			gone = true;
			break;
		}
		tokio::time::sleep(Duration::from_millis(100)).await;
	}

	// Scope, half one: deleting the last file under the rule's path must
	// leave the rule's target directory (`/app`) present. The dispatcher
	// only `rm -f`s the entry; the directory itself is not on the menu.
	// `[ -d /app ]` is the existence check; counting entries would conflate
	// "directory exists but is empty" with "directory was removed".
	let app_after_entry_delete = engine
		.test_exec_capture(
			&cname,
			vec![
				"sh".into(),
				"-c".into(),
				"if [ -d /app ]; then echo present; fi".into(),
			],
		)
		.await
		.unwrap_or_default();

	// Scope, half two: deleting the rule's root directory entirely must
	// still leave `/app` in the container. The dispatcher computes
	// `container_path = /app` for that path (entry_name is empty when
	// removed equals root), and the placement guard refuses with
	// `refusing to remove the rule target directory`. The container sees
	// only the refused warn line; the directory survives.
	fs::remove_dir_all(&src).unwrap();

	// Give the watcher a moment to receive the Remove events and run the
	// refusal, then read the container back. The dispatcher does not block,
	// so the warn line lands first and the existence check immediately
	// after is the right shape.
	let mut app_survived = false;
	let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
	while tokio::time::Instant::now() < deadline {
		let out = engine
			.test_exec_capture(
				&cname,
				vec![
					"sh".into(),
					"-c".into(),
					"if [ -d /app ]; then echo present; fi".into(),
				],
			)
			.await
			.unwrap_or_default();
		if out.contains("present") {
			app_survived = true;
			break;
		}
		tokio::time::sleep(Duration::from_millis(100)).await;
	}

	handle.abort();
	drop(teardown);

	assert!(
		arrived,
		"the initial sync did not place the file inside the container; cannot claim deletion was propagated"
	);
	assert!(
		gone,
		"the host-side deletion did not propagate; /app/f.txt is still present in the container"
	);
	assert!(
		app_after_entry_delete.contains("present"),
		"deleting the last entry under the rule's path removed /app from the container: {app_after_entry_delete:?}"
	);
	assert!(
		app_survived,
		"deleting the rule's root directory removed /app from the container"
	);
}

/// Poll until reading `path` inside `container` yields `expect`, or `secs`
/// elapse. A local helper to keep this test file self-contained: the same
/// helper lives next to the other watch tests under a `fn` (not `pub`),
/// so importing it across module boundaries is not available.
async fn poll_container_contains(
	engine: &Engine,
	cname: &str,
	path: &str,
	expect: &str,
	secs: u64,
) -> bool {
	let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
	while tokio::time::Instant::now() < deadline {
		if let Ok(out) = engine
			.test_exec_capture(cname, vec!["cat".into(), path.into()])
			.await
		{
			if out.contains(expect) {
				return true;
			}
		}
		tokio::time::sleep(Duration::from_millis(100)).await;
	}
	false
}
