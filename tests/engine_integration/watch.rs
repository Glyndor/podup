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
	// Give watch() time to run initial_sync before aborting
	tokio::time::sleep(Duration::from_millis(300)).await;
	handle.abort();

	engine.down(&file).await.unwrap();
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

	tokio::time::sleep(Duration::from_millis(150)).await;
	fs::write(watch_dir.join("main.txt"), b"v2").unwrap();
	tokio::time::sleep(Duration::from_millis(400)).await;

	handle.abort();
	engine.down(&file).await.unwrap();
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

	tokio::time::sleep(Duration::from_millis(150)).await;
	fs::write(watch_dir.join("main.txt"), b"v2").unwrap();
	tokio::time::sleep(Duration::from_millis(400)).await;

	handle.abort();
	engine.down(&file).await.unwrap();
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

	tokio::time::sleep(Duration::from_millis(150)).await;
	fs::write(watch_dir.join("main.txt"), b"v2").unwrap();
	tokio::time::sleep(Duration::from_millis(400)).await;

	handle.abort();
	engine.down(&file).await.unwrap();
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

	// Modify a file to trigger the event dispatch path
	tokio::time::sleep(Duration::from_millis(150)).await;
	fs::write(watch_dir.join("app.txt"), b"v2").unwrap();
	tokio::time::sleep(Duration::from_millis(400)).await;

	watch_handle.abort();
	engine.down(&file).await.unwrap();
}
