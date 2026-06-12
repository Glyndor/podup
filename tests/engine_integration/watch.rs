//! Watch integration tests (require the test-helpers feature).
use std::time::Duration;

use super::*;

#[tokio::test]
async fn watch_no_develop_rules_returns_immediately() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("wno");
	let engine = Engine::new(client, proj.clone());
	// No develop.watch section → watch() returns Ok(()) immediately
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.watch(&file).await.unwrap();
}

#[tokio::test]
async fn watch_sync_file_to_container() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let src_file = dir.path().join("app.txt");
	fs::write(&src_file, b"initial content").unwrap();

	let proj = proj("wsy");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine
		.test_sync_to_container(&format!("{proj}-web"), &src_file, "/tmp/app.txt")
		.await
		.unwrap();
	engine.down(&file).await.unwrap();
}

#[tokio::test]
async fn watch_restart_container() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("wrs");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine
		.test_watch_restart(&format!("{proj}-web"))
		.await
		.unwrap();
	engine.down(&file).await.unwrap();
}

#[tokio::test]
async fn watch_exec_in_container() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("wex");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine
		.test_watch_exec(
			&format!("{proj}-web"),
			vec!["echo".to_string(), "from-watch-exec".to_string()],
		)
		.await
		.unwrap();
	engine.down(&file).await.unwrap();
}

#[tokio::test]
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
	let handle = tokio::spawn(async move { engine2.watch(&file2).await });

	// Poll for the observable effect of initial_sync (the file appearing in the
	// container) instead of sleeping a fixed duration and assuming it ran.
	let cname = format!("{proj}-web");
	let synced = poll_synced(&engine, &cname, "/tmp/app.txt", "initial", 15).await;

	handle.abort();
	engine.down(&file).await.unwrap();
	assert!(synced, "initial_sync did not copy the file into the container");
}

#[tokio::test]
async fn watch_restart_action_via_event() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let watch_dir = dir.path().join("src");
	fs::create_dir(&watch_dir).unwrap();
	fs::write(watch_dir.join("main.txt"), b"v1").unwrap();

	let proj = proj("wra");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    develop:\n      watch:\n        - path: src\n          action: restart\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();

	let client2 = podup::podman::connect_from_env()
		.or_else(|_| podup::podman::connect(None))
		.unwrap();
	let engine2 = Engine::with_base_dir(client2, proj.clone(), dir.path().to_path_buf());
	let file2 = file.clone();
	let handle = tokio::spawn(async move { engine2.watch(&file2).await });

	// PID 1 is `sleep infinity`; a restart replaces it, so its /proc/1/stat
	// (which carries the process start time) changes. Poll for that change
	// rather than sleeping and hoping the restart happened. The watched file is
	// re-touched periodically to beat the watcher-registration race; a restart
	// stop has a 5s grace, so triggers are spaced out.
	let cname = format!("{proj}-web");
	let before = engine
		.test_exec_capture(&cname, vec!["cat".into(), "/proc/1/stat".into()])
		.await
		.unwrap_or_default();
	let restarted = poll_restarted(&engine, &cname, &watch_dir.join("main.txt"), &before, 45).await;

	handle.abort();
	engine.down(&file).await.unwrap();
	assert!(
		before.is_empty() || restarted,
		"watched change did not restart the container"
	);
}

#[tokio::test]
async fn watch_sync_and_restart_action_via_event() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let watch_dir = dir.path().join("src");
	fs::create_dir(&watch_dir).unwrap();
	fs::write(watch_dir.join("main.txt"), b"v1").unwrap();

	let proj = proj("wsr");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    develop:\n      watch:\n        - path: src\n          action: sync+restart\n          target: /app/\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();

	let client2 = podup::podman::connect_from_env()
		.or_else(|_| podup::podman::connect(None))
		.unwrap();
	let engine2 = Engine::with_base_dir(client2, proj.clone(), dir.path().to_path_buf());
	let file2 = file.clone();
	let handle = tokio::spawn(async move { engine2.watch(&file2).await });

	// The synced file lands at /app/main.txt and survives the restart (the
	// writable layer is preserved). Poll for it; restart's 5s stop grace means
	// triggers are spaced out.
	let cname = format!("{proj}-web");
	let synced =
		poll_synced_spaced(&engine, &cname, &watch_dir.join("main.txt"), "/app/main.txt", 45).await;

	handle.abort();
	engine.down(&file).await.unwrap();
	assert!(synced, "sync+restart did not sync the file into the container");
}

#[tokio::test]
async fn watch_sync_and_exec_action_via_event() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let watch_dir = dir.path().join("src");
	fs::create_dir(&watch_dir).unwrap();
	fs::write(watch_dir.join("main.txt"), b"v1").unwrap();

	let proj = proj("wse");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    develop:\n      watch:\n        - path: src\n          action: sync+exec\n          target: /app/\n          exec:\n            command: [\"echo\", \"reloaded\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();

	let client2 = podup::podman::connect_from_env()
		.or_else(|_| podup::podman::connect(None))
		.unwrap();
	let engine2 = Engine::with_base_dir(client2, proj.clone(), dir.path().to_path_buf());
	let file2 = file.clone();
	let handle = tokio::spawn(async move { engine2.watch(&file2).await });

	// Observe the sync half of sync+exec: the file reaching /app/main.txt.
	let cname = format!("{proj}-web");
	let synced = poll_synced(&engine, &cname, "/app/main.txt", "v2", 15).await;

	handle.abort();
	engine.down(&file).await.unwrap();
	assert!(synced, "sync+exec did not sync the file into the container");
}

#[tokio::test]
async fn watch_event_loop_dispatches_sync() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let watch_dir = dir.path().join("src");
	fs::create_dir(&watch_dir).unwrap();
	fs::write(watch_dir.join("app.txt"), b"v1").unwrap();

	let proj = proj("wev");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let rel_path = "src";
	let yaml = format!(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    develop:\n      watch:\n        - path: {rel_path}\n          action: sync\n          target: /app/\n"
	);
	let file = parse_str(&yaml).unwrap();

	engine.up(&file).await.unwrap();

	let client2 = podup::podman::connect_from_env()
		.or_else(|_| podup::podman::connect(None))
		.unwrap();
	let engine2 = Engine::with_base_dir(client2, proj.clone(), dir.path().to_path_buf());
	let file2 = file.clone();
	let watch_handle = tokio::spawn(async move { engine2.watch(&file2).await });

	// Poll for the dispatched sync's effect: src/app.txt reaching /app/app.txt.
	let cname = format!("{proj}-web");
	let synced =
		poll_synced_writing(&engine, &cname, &watch_dir.join("app.txt"), "/app/app.txt", 15).await;

	watch_handle.abort();
	engine.down(&file).await.unwrap();
	assert!(synced, "event loop did not dispatch the sync into the container");
}

/// Poll until `cat`-ing `path` in the container yields `expect`, or `secs`
/// elapse. Read-only: used when the trigger already happened (initial_sync).
async fn poll_synced(engine: &Engine, cname: &str, path: &str, expect: &str, secs: u64) -> bool {
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

/// Re-write `src` to "v2" every iteration (idempotent, beats the registration
/// race) and poll until it appears at `cpath` in the container. For fast
/// actions (sync, sync+exec) where re-triggering is cheap.
async fn poll_synced_writing(
	engine: &Engine,
	cname: &str,
	src: &std::path::Path,
	cpath: &str,
	secs: u64,
) -> bool {
	let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
	while tokio::time::Instant::now() < deadline {
		fs::write(src, b"v2").unwrap();
		if let Ok(out) = engine
			.test_exec_capture(cname, vec!["cat".into(), cpath.into()])
			.await
		{
			if out.contains("v2") {
				return true;
			}
		}
		tokio::time::sleep(Duration::from_millis(100)).await;
	}
	false
}

/// Like [`poll_synced_writing`] but re-triggers only every 8s, for actions
/// whose restart carries a 5s stop grace and must not be flapped continuously.
async fn poll_synced_spaced(
	engine: &Engine,
	cname: &str,
	src: &std::path::Path,
	cpath: &str,
	secs: u64,
) -> bool {
	let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
	fs::write(src, b"v2").unwrap();
	let mut last_trigger = tokio::time::Instant::now();
	while tokio::time::Instant::now() < deadline {
		if last_trigger.elapsed() >= Duration::from_secs(8) {
			fs::write(src, b"v2").unwrap();
			last_trigger = tokio::time::Instant::now();
		}
		if let Ok(out) = engine
			.test_exec_capture(cname, vec!["cat".into(), cpath.into()])
			.await
		{
			if out.contains("v2") {
				return true;
			}
		}
		tokio::time::sleep(Duration::from_millis(200)).await;
	}
	false
}

/// Trigger a restart by touching `src` (spaced every 8s for the stop grace) and
/// poll until PID 1's /proc/1/stat differs from `before`, proving a restart.
async fn poll_restarted(
	engine: &Engine,
	cname: &str,
	src: &std::path::Path,
	before: &str,
	secs: u64,
) -> bool {
	if before.is_empty() {
		return false;
	}
	let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
	fs::write(src, b"v2").unwrap();
	let mut last_trigger = tokio::time::Instant::now();
	let mut counter = 2u32;
	while tokio::time::Instant::now() < deadline {
		if last_trigger.elapsed() >= Duration::from_secs(8) {
			counter += 1;
			fs::write(src, counter.to_string().as_bytes()).unwrap();
			last_trigger = tokio::time::Instant::now();
		}
		if let Ok(now) = engine
			.test_exec_capture(cname, vec!["cat".into(), "/proc/1/stat".into()])
			.await
		{
			if !now.is_empty() && now != before {
				return true;
			}
		}
		tokio::time::sleep(Duration::from_millis(200)).await;
	}
	false
}
