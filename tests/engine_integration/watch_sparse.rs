//! A watched tree holding a file with holes (requires the test-helpers feature).
//!
//! This is the path where #1775 was silent. A failed sync is a `warn!` and the
//! loop is meant to survive one, so `watch` kept running, kept saying it was
//! watching, exited 0, and the file never reached the container. An exit code
//! proves nothing here: the assertion has to be that the bytes arrived.
use std::time::Duration;

use super::*;

const HOLE: u64 = 1 << 20;

#[tokio::test]
async fn watch_initial_sync_carries_a_sparse_file() {
	use std::io::{Seek, SeekFrom, Write};
	use std::os::unix::fs::MetadataExt;

	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let src = dir.path().join("src");
	fs::create_dir(&src).unwrap();
	fs::write(src.join("plain.txt"), b"beside-the-hole").unwrap();

	let holey = src.join("holey.bin");
	let mut f = fs::File::create(&holey).unwrap();
	f.set_len(HOLE).unwrap();
	// `set_len` leaves the cursor at 0, so the seek is what puts the tail at the
	// end rather than at the start.
	f.seek(SeekFrom::Start(HOLE)).unwrap();
	f.write_all(b"tail").unwrap();
	f.sync_all().unwrap();
	drop(f);
	let meta = fs::metadata(&holey).unwrap();
	if meta.blocks() * 512 >= meta.size() {
		eprintln!("watch sparse: no hole on this filesystem, nothing measured");
		return;
	}

	let proj = proj("wsparse");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    develop:\n      watch:\n        - path: src\n          action: sync\n          target: /app\n          initial_sync: true\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();

	let client2 = podup::podman::connect_from_env()
		.or_else(|_| podup::podman::connect(None))
		.unwrap();
	let engine2 = Engine::with_base_dir(client2, proj.clone(), dir.path().to_path_buf());
	let file2 = file.clone();
	let handle = tokio::spawn(async move { engine2.watch(&file2).await });

	// The size of the arrived file, not merely its presence: a transfer that
	// succeeded and truncated the expanded holes would pass a `test -f`.
	let cname = format!("{proj}-web-1");
	let want = (HOLE + 4).to_string();
	let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
	let mut arrived = false;
	while tokio::time::Instant::now() < deadline {
		if let Ok(out) = engine
			.test_exec_capture(
				&cname,
				vec![
					"sh".into(),
					"-c".into(),
					"cat /app/plain.txt && stat -c %s /app/holey.bin".into(),
				],
			)
			.await
		{
			if out.contains("beside-the-hole") && out.contains(&want) {
				arrived = true;
				break;
			}
		}
		tokio::time::sleep(Duration::from_millis(200)).await;
	}

	handle.abort();
	engine.down(&file).await.unwrap();
	assert!(
		arrived,
		"the initial sync of a tree holding a sparse file never delivered it; \
		 this failure is a warning inside watch, so nothing else reports it"
	);
}
