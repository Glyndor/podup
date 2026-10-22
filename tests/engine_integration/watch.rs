//! Watch integration tests driving the full loop: the initial sync, and the
//! rebuild and sync+restart actions a watched change triggers (require the
//! test-helpers feature).
use std::fs;
use std::time::Duration;

use super::*;
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_initial_sync_runs() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let src = dir.path().join("app.txt");
	fs::write(&src, b"initial").unwrap();

	let proj = proj("wis");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    develop:\n      watch:\n        - path: app.txt\n          action: sync\n          target: /tmp/app.txt\n          initial_sync: true\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();

	let client2 = podup::podman::connect_from_env()
		.or_else(|_| podup::podman::connect(None))
		.unwrap();
	let engine2 = Engine::with_base_dir(client2, proj.clone(), dir.path().to_path_buf());
	let file2 = file.clone();
	let mut handle = tokio::spawn(async move { engine2.watch(&file2).await });

	// Poll for the observable effect of initial_sync (the file appearing in the
	// container) instead of sleeping a fixed duration and assuming it ran.
	// Routed through `poll_with_watch` so a watch task that died (e.g. an
	// inotify exhaustion surfacing as `Err(Watch)`) panics with the watch
	// error instead of waiting out the deadline and blaming the sync.
	let cname = format!("{proj}-web-1");
	let synced = poll_with_watch(&mut handle, Duration::from_secs(60), || {
		poll_synced_once(&engine, &cname, "/tmp/app.txt", "initial")
	})
	.await;

	handle.abort();
	engine.down(&file).await.unwrap();
	assert!(
		synced,
		"initial_sync did not copy the file into the container"
	);
}

/// `rebuild` was the one watch action with no coverage at all, and it is the
/// only one that goes all the way back through `build` and container recreation
/// rather than touching a running container. It reads its trigger file into the
/// image, so a rebuild that silently did nothing, or that rebuilt the image and
/// left the old container running, is visible as stale content.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_rebuild_recreates_the_container_from_the_new_image() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	fs::write(dir.path().join("app.txt"), b"v1").unwrap();
	fs::write(
		dir.path().join("Dockerfile"),
		b"FROM alpine:latest\nCOPY app.txt /app.txt\nCMD [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	let proj = proj("wrb");
	// `down` leaves the rebuilt image behind; the guard removes it even when
	// an assertion below panics.
	let _image = TestImage::new(format!("{proj}-web"));
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let file = parse_str(
		"services:\n  web:\n    build: .\n    develop:\n      watch:\n        - path: app.txt\n          action: rebuild\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	let cname = format!("{proj}-web-1");
	// The image really did carry v1 before the change, so a stale read later means
	// the rebuild did not happen rather than the fixture never being right.
	let before = poll_synced(&engine, &cname, "/app.txt", "v1", 30).await;

	let client2 = podup::podman::connect_from_env()
		.or_else(|_| podup::podman::connect(None))
		.unwrap();
	let engine2 = Engine::with_base_dir(client2, proj.clone(), dir.path().to_path_buf());
	let file2 = file.clone();
	let mut handle = tokio::spawn(async move { engine2.watch(&file2).await });

	// Give the watcher a moment to register before changing the file, then poll
	// for the effect rather than assuming a fixed rebuild duration. Routed
	// through `poll_with_watch` so a watch task that died panics with the
	// watch error instead of blaming the rebuild.
	tokio::time::sleep(Duration::from_secs(2)).await;
	fs::write(dir.path().join("app.txt"), b"v2").unwrap();
	let rebuilt = poll_with_watch(&mut handle, Duration::from_secs(120), || {
		poll_synced_once(&engine, &cname, "/app.txt", "v2")
	})
	.await;

	handle.abort();
	let containers = engine
		.test_project_container_names()
		.await
		.unwrap_or_default();
	engine.down(&file).await.unwrap();

	assert!(before, "the image did not carry v1 before the change");
	assert!(
		rebuilt,
		"the container still serves the old image, so rebuild did not recreate it"
	);
	assert_eq!(
		containers.len(),
		1,
		"rebuild left the previous container behind: {containers:?}"
	);
}

/// `sync+restart` is two effects in one action, and a test that only checks the
/// file arrived would pass with the restart half missing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_sync_and_restart_does_both() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	fs::write(dir.path().join("app.txt"), b"synced-value").unwrap();

	let proj = proj("wsr");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	// The command appends one line per start. A restart re-runs it against the
	// same (persisting) filesystem, so the line count is a container-scoped
	// counter of how many times the process was started, unlike /proc/uptime,
	// which is not namespaced and would report the host's.
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"echo start >> /starts; sleep infinity\"]\n    develop:\n      watch:\n        - path: app.txt\n          action: sync+restart\n          target: /tmp/app.txt\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	let cname = format!("{proj}-web-1");

	let client2 = podup::podman::connect_from_env()
		.or_else(|_| podup::podman::connect(None))
		.unwrap();
	let engine2 = Engine::with_base_dir(client2, proj.clone(), dir.path().to_path_buf());
	let file2 = file.clone();
	let mut handle = tokio::spawn(async move { engine2.watch(&file2).await });

	tokio::time::sleep(Duration::from_secs(2)).await;
	fs::write(dir.path().join("app.txt"), b"changed-value").unwrap();
	let synced = poll_with_watch(&mut handle, Duration::from_secs(60), || {
		poll_synced_once(&engine, &cname, "/tmp/app.txt", "changed-value")
	})
	.await;

	// Poll for the second start line rather than sleeping and hoping. Both
	// polls share the same handle: the helper's per-tick `is_finished()`
	// check fires on either one if the watcher dies between them.
	let restarted = poll_with_watch(&mut handle, Duration::from_secs(60), || {
		poll_synced_once(&engine, &cname, "/starts", "start\nstart")
	})
	.await;

	handle.abort();
	engine.down(&file).await.unwrap();
	assert!(synced, "sync+restart did not copy the file");
	assert!(
		restarted,
		"sync+restart copied the file but never restarted the container"
	);
}
