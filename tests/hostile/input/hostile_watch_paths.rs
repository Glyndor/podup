//! A link at the event root and an ignored FIFO must not escape or block sync.
#![cfg(all(unix, feature = "watch", feature = "test-helpers"))]

use std::fs;
use std::process::Command;

#[path = "../support/fake_engine.rs"]
mod fake_engine;
#[path = "../support/host_files.rs"]
mod host_files;

use fake_engine::FakeEngine;
use host_files::{accepts, entries, partial_archive_bytes, Running, BUDGET, MARKER, SAFE};

#[tokio::test]
async fn symlink_event_root_is_refused_and_uploads_nothing() {
	let fake = FakeEngine::start(accepts);
	let outside = tempfile::tempdir().unwrap();
	fs::write(outside.path().join("marker"), MARKER).unwrap();
	let root = fake.dir().join("event");
	fs::create_dir(&root).unwrap();
	fs::rename(&root, fake.dir().join("old-event")).unwrap();
	std::os::unix::fs::symlink(outside.path(), &root).unwrap();
	let socket = fake.dir().join("podman.sock");
	let client = podup::podman::connect(Some(socket.to_str().unwrap())).unwrap();
	let engine = podup::Engine::with_base_dir(client, "hostile".into(), fake.dir().into());
	let result = tokio::time::timeout(BUDGET, engine.test_sync_to_container("web", &root, "/dst"))
		.await
		.expect("symlink sync blocked");
	assert!(result.is_err(), "symlink event root accepted");
	let error = result.unwrap_err().to_string();
	assert!(
		error.contains("sync:") && (error.contains("path") || error.contains("link")),
		"{error}"
	);
	for request in fake.requests().iter().filter(|r| r.starts_with(b"PUT ")) {
		assert!(
			!partial_archive_bytes(request)
				.windows(MARKER.len())
				.any(|bytes| bytes == MARKER.as_bytes()),
			"outside marker uploaded"
		);
	}
}

/// The watch rule ignores a pipe in the tree. The filter is the walker's own, so
/// the pipe is never opened and the initial sync still sends the other files.
#[test]
fn excluded_fifo_does_not_stop_the_initial_sync() {
	let fake = FakeEngine::start(accepts);
	fs::create_dir(fake.dir().join("src")).unwrap();
	fs::write(fake.dir().join("src/keep.txt"), SAFE).unwrap();
	assert!(Command::new("mkfifo")
		.arg(fake.dir().join("src/pipe"))
		.status()
		.unwrap()
		.success());
	fs::write(fake.dir().join("compose.yaml"), "services:\n  web:\n    image: alpine\n    develop:\n      watch:\n        - action: sync\n          path: ./src\n          target: /dst\n          initial_sync: true\n          ignore: [pipe]\n").unwrap();
	let mut child = Running::start(&fake, &["watch"]);
	child.until("watching 1 rule(s)");
	let text = child.text();
	assert!(
		!text.contains("initial sync failed"),
		"excluded FIFO broke sync: {text}"
	);
	assert!(text.contains("synced "), "no successful sync: {text}");
	let requests = fake.requests();
	let upload = requests
		.iter()
		.rev()
		.find(|r| r.starts_with(b"PUT "))
		.expect("no sync upload");
	let sent = entries(upload);
	assert!(sent
		.iter()
		.any(|(name, bytes)| name == "keep.txt" && bytes == SAFE.as_bytes()));
	assert!(!sent
		.iter()
		.any(|(name, _)| name.ends_with("pipe") || name.ends_with("marker")));
}
