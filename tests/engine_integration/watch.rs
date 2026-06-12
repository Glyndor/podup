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
	let handle = tokio::spawn(async move { engine2.watch(&file2).await });

	// Poll for the observable effect of initial_sync (the file appearing in the
	// container) instead of sleeping a fixed duration and assuming it ran.
	let cname = format!("{proj}-web");
	let synced = poll_synced(&engine, &cname, "/tmp/app.txt", "initial", 60).await;

	handle.abort();
	engine.down(&file).await.unwrap();
	assert!(
		synced,
		"initial_sync did not copy the file into the container"
	);
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
