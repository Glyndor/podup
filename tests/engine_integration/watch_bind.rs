//! Live tests for the watch engine's interaction with bind mounts and named
//! volumes: a carve-out under a self-bind must still copy, and a rule whose
//! root writes back through the only bind it has must drop every sync so the
//! loop cannot fire (#1985). The maps-back test uses a named volume at
//! `/app/cache` as a safe carve-out under the `"./src:/app"` self-bind; a
//! probe written into that carve-out before and after the unsafe changes
//! proves the watcher handled the events instead of sitting quiet.
#![cfg(feature = "test-helpers")]

use std::time::Duration;

use super::*;

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
				.name("watch_bind_wsvnc_teardown".into())
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
				eprintln!("watch_bind wsvnc teardown: down failed: {e}");
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
	let volume_key = format!("cache_{proj}");
	let volume_name = format!("{proj}_{volume_key}");
	let compose_body = format!(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    volumes:\n      - \"./src:/app\"\n      - \"{volume_key}:/app/cache\"\n    develop:\n      watch:\n        - path: .\n          action: sync\n          target: /app\n          initial_sync: false\nvolumes:\n  {volume_key}:\n"
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
				.name("watch_bind_wdcnb_teardown".into())
				.spawn(move || {
					let rt = tokio::runtime::Builder::new_current_thread()
						.enable_all()
						.build()
						.expect("teardown runtime");
					rt.block_on(async move {
						engine.down(&file).await?;
						let _ = podman_cmd_volume_rm(&socket, &volume_name);
						Ok::<(), podup::ComposeError>(())
					})
				})
				.expect("teardown thread")
				.join()
				.expect("teardown thread result");
			if let Err(e) = res {
				eprintln!("watch_bind wdcnb teardown: down failed: {e}");
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

	// The `cache/` directory must exist before the watch task starts so
	// notify's recursive walker adds an inotify watch for it during
	// setup. A directory created after the initial walk can race the
	// file create event and lose it.
	fs::create_dir_all(dir.path().join("cache")).unwrap();

	// Give the watch task a moment to register the inotify watches and
	// enter its event loop before generating the first event. Without
	// this the `fs::write` below can complete before the watcher's
	// inotify fd is open, and the event is lost.
	tokio::time::sleep(Duration::from_millis(500)).await;

	// Step 1: prove the watcher is live before the unsafe changes. A probe
	// under /app/cache reaches the container only through the watcher's
	// own write to the carve-out; the per-entry filter does not skip it
	// because the longer mount at /app/cache is a non-bind named volume
	// and writes_back returns None for it. The write is performed first
	// so the first poll tick has a live inotify event to debounce. The
	// probe is written in the project dir (not in `src/`) because the
	// bind `./src:/app` would otherwise map the host path to
	// `/app/src/cache/probe1.txt`, which is under the bind, not the
	// volume.
	fs::write(dir.path().join("cache").join("probe1.txt"), b"1").unwrap();
	let probe1_seen = poll_with_watch(&mut handle, Duration::from_secs(30), || {
		poll_container_contains_once(&engine, &cname, "/app/cache/probe1.txt", "1")
	})
	.await;

	// Step 2: the unsafe changes. A copy would land at /app/a.txt (host
	// src/a.txt) and a removal would run rm /app/b.txt (host src/b.txt).
	fs::write(dir.path().join("a.txt"), b"hello").unwrap();
	fs::remove_file(dir.path().join("b.txt")).unwrap();

	// Step 3: prove the watcher is still live after the unsafe changes.
	// A second probe reaching the container means the events after the
	// unsafe ones were handled, so the unsafe ones were handled too.
	// Without this the test only proves the watch task is alive; with
	// it the test proves the dispatch loop processed the unsafe events.
	fs::write(dir.path().join("cache").join("probe2.txt"), b"2").unwrap();
	let probe2_seen = poll_with_watch(&mut handle, Duration::from_secs(30), || {
		poll_container_contains_once(&engine, &cname, "/app/cache/probe2.txt", "2")
	})
	.await;

	handle.abort();
	drop(teardown);

	assert!(
		probe1_seen,
		"the first probe did not reach /app/cache/probe1.txt; cannot claim the watcher is live before the unsafe changes"
	);
	assert!(
		probe2_seen,
		"the second probe did not reach /app/cache/probe2.txt; the events after the unsafe changes were not handled, so the unsafe ones were not handled either"
	);
	// Step 4: the existing assertions, with the new evidence in hand. The
	// copy must not have landed: neither `src/a.txt` (the bind's mapped
	// host path) nor `src/src` (a copy of the project dir into itself
	// through the bind) may exist on the host.
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

/// One attempt of reading `path` inside `container` and checking its
/// contents. The same predicate the batch tests use: a local copy so
/// `pub(super)` helpers in the parent module stay private. Named `_once`
/// so the caller knows it does no looping: `poll_with_watch` owns the
/// retry/timeout, so a single attempt is what each tick passes in.
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
