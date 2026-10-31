//! Watch integration tests for the sync copy itself (require the test-helpers
//! feature).
use std::fs;

use super::*;
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
	let cname = format!("{proj}-web-1");
	engine
		.test_sync_to_container(&cname, &src_file, "/tmp/app.txt")
		.await
		.unwrap();
	let first = engine
		.test_exec_capture(&cname, vec!["cat".into(), "/tmp/app.txt".into()])
		.await
		.unwrap_or_default();

	// Sync the same path again with different bytes. A watch rule fires on every
	// change to the path it covers, so a second copy over an existing file is the
	// ordinary case rather than an edge one, and it is the half a first-copy-only
	// assertion cannot see.
	fs::write(&src_file, b"changed content").unwrap();
	// The result is captured instead of unwrapped, deliberately. On Podman 6 this
	// second copy came back as the #1097 could-not-confirm error, which says in
	// its own text that the copy may or may not have landed. #1270 owns that
	// error-reporting defect. What this test owns is whether the bytes arrived,
	// so it reads them and reports what the sync claimed alongside, which is
	// also the measurement #1270 needs to tell its branches apart.
	let reported = engine
		.test_sync_to_container(&cname, &src_file, "/tmp/app.txt")
		.await;
	let second = engine
		.test_exec_capture(&cname, vec!["cat".into(), "/tmp/app.txt".into()])
		.await
		.unwrap_or_default();

	engine.down(&file).await.unwrap();
	assert_eq!(
		first.trim(),
		"initial content",
		"sync returned success but the file never reached the container"
	);
	assert_eq!(
		second.trim(),
		"changed content",
		"the second sync left the first copy in place; it reported {reported:?}"
	);
}

#[tokio::test]
async fn watch_sync_creates_missing_target_directory() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let src_file = dir.path().join("app.txt");
	fs::write(&src_file, b"created").unwrap();

	let proj = proj("wcd");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	let cname = format!("{proj}-web-1");
	// Sync into a directory that does not exist in the image; podup must create
	// it (like docker compose watch) rather than fail.
	engine
		.test_sync_to_container(&cname, &src_file, "/newdir/app.txt")
		.await
		.unwrap();
	let out = engine
		.test_exec_capture(&cname, vec!["cat".into(), "/newdir/app.txt".into()])
		.await
		.unwrap_or_default();
	engine.down(&file).await.unwrap();
	assert!(
		out.contains("created"),
		"sync did not create the missing target directory: {out:?}"
	);
}
