//! Live test: the watch engine must dispatch a single debounce batch
//! correctly when one path is written and removed inside the debounce
//! window, and when one path is removed while another is updated in
//! the same window. The original dispatch took the batch's first
//! event's kind as the dominant kind, which made both shapes apply
//! wrong: a write+remove of the same file became an upload of a
//! missing path; a remove(a)+modify(b) became a remove(b). The
//! current dispatch asks the host filesystem per path instead
//! (#1984).
#![cfg(feature = "test-helpers")]

use std::time::Duration;

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_removal_right_after_a_write_reaches_the_container() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let src = dir.path().join("src");
	fs::create_dir(&src).unwrap();
	let src_file = src.join("f.txt");

	// Pre-spawn write: ensures the initial sync has something to put
	// in the container, so the test's "removal half" has a baseline to
	// compare against. Without it, the deletion would race the initial
	// sync and the assertion could pass for the wrong reason.
	fs::write(&src_file, b"one").unwrap();

	let proj = proj("wbrw");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    develop:\n      watch:\n        - path: ./src\n          action: sync\n          target: /app\n          initial_sync: true\n",
	)
	.unwrap();
	engine.up(&file).await.unwrap();

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
				.name("watch_batch_wbrw_teardown".into())
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
				eprintln!("watch_batch wbrw teardown: down failed: {e}");
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

	let client2 = podup::podman::connect_from_env()
		.or_else(|_| podup::podman::connect(None))
		.unwrap();
	let engine2 = Engine::with_base_dir(client2, proj.clone(), dir.path().to_path_buf());
	let file2 = file.clone();
	let mut handle = tokio::spawn(async move { engine2.watch(&file2).await });

	// Half one: initial sync placed "one" in the container. Poll for
	// it via `poll_with_watch` so a watch task that died (e.g. an
	// inotify exhaustion) panics with the watch error instead of
	// blaming the sync.
	let arrived = poll_with_watch(&mut handle, Duration::from_secs(30), || {
		poll_container_contains_once(&engine, &cname, "/app/f.txt", "one")
	})
	.await;

	// Half two: with no sleep between them, write "two" and remove
	// the file. The watcher must collapse both into one batch (the
	// 100 ms debounce), then dispatch the path as a remove because
	// the host filesystem no longer has the file when the dispatch
	// runs. The old behaviour took the batch's first event's kind
	// (Modify) and tried to upload a file that was already gone.
	fs::write(&src_file, b"two").unwrap();
	fs::remove_file(&src_file).unwrap();

	let gone = poll_with_watch(&mut handle, Duration::from_secs(30), || async {
		container_path_present(&engine, &cname, "/app/f.txt").await == Some(false)
	})
	.await;

	handle.abort();
	drop(teardown);

	assert!(
		arrived,
		"the initial sync did not place f.txt inside the container; cannot claim the removal half was tested"
	);
	assert!(
		gone,
		"the write+remove inside one debounce window left /app/f.txt inside the container; the batch was applied as an upload of a missing path"
	);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_mixed_batch_removes_one_file_and_updates_another() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let src = dir.path().join("src");
	fs::create_dir(&src).unwrap();
	let src_a = src.join("a.txt");
	let src_b = src.join("b.txt");
	fs::write(&src_a, b"a1").unwrap();
	fs::write(&src_b, b"b1").unwrap();

	let proj = proj("wbmx");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    develop:\n      watch:\n        - path: ./src\n          action: sync\n          target: /app\n          initial_sync: true\n",
	)
	.unwrap();
	engine.up(&file).await.unwrap();

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
				.name("watch_batch_wbmx_teardown".into())
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
				eprintln!("watch_batch wbmx teardown: down failed: {e}");
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

	let client2 = podup::podman::connect_from_env()
		.or_else(|_| podup::podman::connect(None))
		.unwrap();
	let engine2 = Engine::with_base_dir(client2, proj.clone(), dir.path().to_path_buf());
	let file2 = file.clone();
	let mut handle = tokio::spawn(async move { engine2.watch(&file2).await });

	// Half one: both files land in the container through the initial
	// sync. Poll for both at once through a single `poll_with_watch`
	// tick; the helper's per-tick `is_finished()` still fires if the
	// watch task dies between this poll and the next.
	let both_present = poll_with_watch(&mut handle, Duration::from_secs(30), || async {
		let a = poll_container_contains_once(&engine, &cname, "/app/a.txt", "a1").await;
		let b = poll_container_contains_once(&engine, &cname, "/app/b.txt", "b1").await;
		a && b
	})
	.await;

	// Half two: remove a.txt and write b.txt with no sleep in
	// between. The watcher must collapse both into one debounce
	// batch and dispatch per path: a.txt becomes a removal (the
	// host no longer has it), b.txt becomes an upload of "b2".
	// The old behaviour took the batch's first event's kind
	// (Remove) and tried to remove b.txt instead.
	fs::remove_file(&src_a).unwrap();
	fs::write(&src_b, b"b2").unwrap();

	let a_gone = poll_with_watch(&mut handle, Duration::from_secs(30), || async {
		container_path_present(&engine, &cname, "/app/a.txt").await == Some(false)
	})
	.await;
	let b_updated = poll_with_watch(&mut handle, Duration::from_secs(30), || {
		poll_container_contains_once(&engine, &cname, "/app/b.txt", "b2")
	})
	.await;

	handle.abort();
	drop(teardown);

	assert!(
		both_present,
		"the initial sync did not place a.txt and b.txt inside the container; cannot claim the mixed-batch half was tested"
	);
	assert!(
		a_gone,
		"the remove(a) half of the batch did not reach the container; /app/a.txt is still present"
	);
	assert!(
		b_updated,
		"the modify(b) half of the batch did not upload the new bytes; /app/b.txt does not contain b2"
	);
}

/// One attempt of reading `path` inside `container` and checking its
/// contents. A local helper to keep this test file self-contained: the
/// same predicate lives next to the other watch tests under a `fn`
/// (not `pub`), so importing it across module boundaries is not
/// available. Named `_once` so the caller knows it does no looping:
/// `poll_with_watch` owns the retry/timeout, so a single attempt is
/// what each tick passes in.
async fn poll_container_contains_once(
	engine: &Engine,
	cname: &str,
	path: &str,
	expect: &str,
) -> bool {
	if let Ok(out) = engine
		.test_exec_capture(cname, vec!["cat".into(), path.into()])
		.await
	{
		out.contains(expect)
	} else {
		false
	}
}

/// Replacing a watched file with an empty directory on the host must turn
/// the same path inside the container into a directory, not keep the old
/// file around. The packer used to ship only the directory's descendants
/// (none for an empty directory), so the tar overwrote nothing and the
/// previous file survived the change (#1985).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_file_replaced_by_empty_directory_becomes_a_directory() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let src = dir.path().join("src");
	fs::create_dir(&src).unwrap();
	let src_file = src.join("f");
	fs::write(&src_file, b"one").unwrap();

	let proj = proj("wbef");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    develop:\n      watch:\n        - path: ./src\n          action: sync\n          target: /app\n          initial_sync: true\n",
	)
	.unwrap();
	engine.up(&file).await.unwrap();

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
				.name("watch_batch_wbef_teardown".into())
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
				eprintln!("watch_batch wbef teardown: down failed: {e}");
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

	let client2 = podup::podman::connect_from_env()
		.or_else(|_| podup::podman::connect(None))
		.unwrap();
	let engine2 = Engine::with_base_dir(client2, proj.clone(), dir.path().to_path_buf());
	let file2 = file.clone();
	let mut handle = tokio::spawn(async move { engine2.watch(&file2).await });

	// Wait for the initial sync to land /app/f with content "one".
	let arrived = poll_with_watch(&mut handle, Duration::from_secs(30), || {
		poll_container_contains_once(&engine, &cname, "/app/f", "one")
	})
	.await;

	// Replace the file with an empty directory, no sleep in between so the
	// watcher sees the two events as one debounce batch.
	fs::remove_file(&src_file).unwrap();
	fs::create_dir(&src_file).unwrap();

	// Poll for /app/f being a directory inside the container.
	let became_dir = poll_with_watch(&mut handle, Duration::from_secs(30), || async {
		let out = engine
			.test_exec_capture(
				&cname,
				vec![
					"sh".into(),
					"-c".into(),
					"if [ -d /app/f ]; then echo dir; else echo other; fi".into(),
				],
			)
			.await
			.unwrap_or_default();
		out.contains("dir")
	})
	.await;

	handle.abort();
	drop(teardown);

	assert!(
		arrived,
		"the initial sync did not place the file inside the container; cannot claim the directory-replace half was tested"
	);
	assert!(
		became_dir,
		"replacing the file with an empty directory did not turn /app/f into a directory in the container"
	);
}
