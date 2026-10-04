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

/// A descendant of a watched directory whose container path lands in a
/// deeper, more specific mount (a named volume) must still sync. The bind
/// `"./src:/app"` covers every container path, but the volume
/// `"<unique>:/app/cache"` covers a strict sub-tree, so a change to
/// `src/cache/f.txt` should land in `/app/cache/f.txt` even though the bind
/// alone would loop. Before #1985 the per-entry filter only ran on the
/// rule root, so this case copied the file into the writable loop and
/// re-fired the watcher.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_syncs_into_a_volume_nested_under_a_self_bind() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let src = dir.path().join("src");
	fs::create_dir(&src).unwrap();
	// A probe file under the carve-out: only the sync can deliver it
	// because the named volume is empty before the watcher writes.
	let probe_host = src.join("cache");
	fs::create_dir(&probe_host).unwrap();
	fs::write(probe_host.join("probe.txt"), b"probe").unwrap();

	// A unique-named volume so two parallel runs do not share a host path.
	let proj = proj("wsvnc");
	let volume_key = format!("cache_{proj}");
	let volume_name = format!("{proj}_{volume_key}");
	let compose_body = format!(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    volumes:\n      - \"./src:/app\"\n      - \"{volume_key}:/app/cache\"\n    develop:\n      watch:\n        - path: ./src\n          action: sync\n          target: /app\n          initial_sync: true\nvolumes:\n  {volume_key}:\n"
	);

	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let file = parse_str(&compose_body).unwrap();
	engine.up(&file).await.unwrap();

	// Drop the named volume in teardown (only the one this test created).
	struct Teardown {
		engine: Engine,
		file: podup::compose::types::ComposeFile,
		volume_name: String,
		socket: String,
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
			let volume_name = std::mem::take(&mut self.volume_name);
			let socket = std::mem::take(&mut self.socket);
			let res = std::thread::Builder::new()
				.name("watch_batch_wsvnc_teardown".into())
				.spawn(move || {
					let rt = tokio::runtime::Builder::new_current_thread()
						.enable_all()
						.build()
						.expect("teardown runtime");
					rt.block_on(async move {
						engine.down(&file).await?;
						// Best-effort: a 404 here means another concurrent run
						// already cleaned it up; any other error is logged
						// rather than failing the test (teardown failures
						// must not change a green into red).
						let _ = podman_cmd_volume_rm(&socket, &volume_name);
						Ok::<(), podup::ComposeError>(())
					})
				})
				.expect("teardown thread")
				.join()
				.expect("teardown thread result");
			if let Err(e) = res {
				eprintln!("watch_batch wsvnc teardown: down failed: {e}");
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
		volume_name: volume_name.clone(),
		socket: podman_socket_url().expect("podman socket"),
	};

	let cname = format!("{proj}-web-1");

	let client2 = podup::podman::connect_from_env()
		.or_else(|_| podup::podman::connect(None))
		.unwrap();
	let engine2 = Engine::with_base_dir(client2, proj.clone(), dir.path().to_path_buf());
	let file2 = file.clone();
	let mut handle = tokio::spawn(async move { engine2.watch(&file2).await });

	// Prove the watcher is live: the initial sync carries the probe file
	// into /app/cache (the carve-out, not the bind) so the container sees
	// the bytes only because the sync ran.
	let probe_arrived = poll_with_watch(&mut handle, Duration::from_secs(30), || {
		poll_container_contains_once(&engine, &cname, "/app/cache/probe.txt", "probe")
	})
	.await;

	// The actual change we want to test is a separate write to a deeper
	// file. The carve-out path only reaches the container through the
	// sync, so the file's arrival proves the per-entry filter accepted it.
	let deeper_host = probe_host.join("f.txt");
	fs::write(&deeper_host, b"vol").unwrap();

	let arrived = poll_with_watch(&mut handle, Duration::from_secs(30), || {
		poll_container_contains_once(&engine, &cname, "/app/cache/f.txt", "vol")
	})
	.await;

	handle.abort();
	drop(teardown);

	assert!(
		probe_arrived,
		"the initial sync did not place the probe file under /app/cache; cannot claim the carve-out was tested"
	);
	assert!(
		arrived,
		"a change to src/cache/f.txt did not reach /app/cache/f.txt; the per-entry filter dropped the carve-out"
	);
}

/// A rule whose root writes back through the only bind it has must drop
/// every entry of every sync, so a write to the project dir does not
/// produce a host-side copy at `src/a.txt`. Every path of the rule maps
/// back through the bind, so the watcher is expected to be quiet, not
/// noisy. The 3 s / 5 s waits use `poll_with_watch` so a watch task that
/// died still panics with its error instead of being mistaken for a quiet
/// loop (#1985).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_does_not_copy_into_a_bind_that_maps_back() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let src = dir.path().join("src");
	fs::create_dir(&src).unwrap();
	// `src/b.txt` is a file of its own that only the bind exposes at
	// `/app/b.txt`; the project's `b.txt` maps to the same container path.
	// Deleting the project's `b.txt` must not delete `src/b.txt`.
	fs::write(src.join("b.txt"), b"keep").unwrap();
	fs::write(dir.path().join("b.txt"), b"project").unwrap();

	let proj = proj("wdcnb");
	let compose_body = "services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    volumes:\n      - \"./src:/app\"\n    develop:\n      watch:\n        - path: .\n          action: sync\n          target: /app\n          initial_sync: false\n"
		.to_string();

	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let file = parse_str(&compose_body).unwrap();
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
				.name("watch_batch_wdcnb_teardown".into())
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
				eprintln!("watch_batch wdcnb teardown: down failed: {e}");
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

	let client2 = podup::podman::connect_from_env()
		.or_else(|_| podup::podman::connect(None))
		.unwrap();
	let engine2 = Engine::with_base_dir(client2, proj.clone(), dir.path().to_path_buf());
	let file2 = file.clone();
	let mut handle = tokio::spawn(async move { engine2.watch(&file2).await });

	// Every path of this rule writes back, so no event-driven probe can
	// prove the watcher is live. Let it run for a moment under
	// `poll_with_watch`, which panics with the watch task's error if the
	// task dies, so a dead watcher is not mistaken for a quiet one.
	poll_with_watch(&mut handle, Duration::from_secs(3), || async { false }).await;

	// A copy of `a.txt` would land at `/app/a.txt`, which is host
	// `src/a.txt`; removing `b.txt` would run `rm /app/b.txt`, which is
	// host `src/b.txt`.
	fs::write(dir.path().join("a.txt"), b"hello").unwrap();
	fs::remove_file(dir.path().join("b.txt")).unwrap();
	poll_with_watch(&mut handle, Duration::from_secs(5), || async { false }).await;

	handle.abort();
	drop(teardown);

	// The copy must not have landed: neither `src/a.txt` (the bind's
	// mapped host path) nor `src/src` (a copy of the project dir into
	// itself through the bind) may exist on the host.
	assert!(
		!dir.path().join("src/a.txt").exists(),
		"a copy landed at host path src/a.txt through the bind mount"
	);
	assert!(
		!dir.path().join("src/src").exists(),
		"the rule's bind mapped the project dir into itself; src/src must not exist"
	);
	assert_eq!(
		fs::read_to_string(dir.path().join("src/b.txt"))
			.ok()
			.as_deref(),
		Some("keep"),
		"removing the project's b.txt deleted src/b.txt through the bind mount"
	);
}

/// Remove a named volume by name through the `podman` CLI. Returns
/// silently on a 404 (already gone) and surfaces any other failure to the
/// caller so a leaking volume never silently turns the test red. Used by
/// the volume-bearing tests so they do not leave a named volume behind
/// after `down` runs (which keeps named volumes by design).
fn podman_cmd_volume_rm(socket: &str, name: &str) -> Result<(), String> {
	let out = std::process::Command::new("podman")
		.args(["--url", socket, "volume", "rm", name])
		.output()
		.map_err(|e| format!("podman volume rm {name}: {e}"))?;
	if out.status.success() {
		return Ok(());
	}
	// A 404 from `volume rm` means the volume was already gone; treat it
	// as success so a parallel run cleaning up after us does not surface
	// as a failure.
	let stderr = String::from_utf8_lossy(&out.stderr);
	if stderr.contains("no such volume") || stderr.contains("not found") {
		Ok(())
	} else {
		Err(format!(
			"`podman --url {socket} volume rm {name}` exited {}: {stderr}",
			out.status
		))
	}
}
