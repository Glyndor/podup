//! `cp` integration tests for a payload carrying a file with holes, on its own and
//! beside an ordinary file in the same directory.
use std::fs;

use super::*;
/// A directory carrying one file with holes, copied to a live container.
///
/// The `cp` endpoint refuses a GNU sparse entry with its own wording,
/// `unrecognized Typeflag S`, and refuses the archive rather than the entry, so
/// the ordinary file travelling beside the sparse one is lost too. The
/// directory branch is the one that matters here: it also creates the
/// destination before the stream is rejected, which reads as success to
/// anything that only checks the path exists. #1775.
#[cfg(all(unix, feature = "test-helpers"))]
#[tokio::test]
async fn engine_cp_uploads_a_directory_holding_a_sparse_file() {
	use std::io::{Seek, SeekFrom, Write};
	use std::os::unix::fs::MetadataExt;

	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	fs::create_dir(&payload).unwrap();
	fs::write(payload.join("plain.txt"), b"beside-the-hole").unwrap();

	let hole: u64 = 1 << 20;
	let holey = payload.join("holey.bin");
	let mut f = fs::File::create(&holey).unwrap();
	f.set_len(hole).unwrap();
	// `set_len` leaves the cursor at 0, so without the seek the tail lands at
	// the start and the file is not the one this test describes.
	f.seek(SeekFrom::Start(hole)).unwrap();
	f.write_all(b"tail").unwrap();
	f.sync_all().unwrap();
	drop(f);
	let meta = fs::metadata(&holey).unwrap();
	if meta.blocks() * 512 >= meta.size() {
		eprintln!("cp sparse: no hole on this filesystem, nothing measured");
		return;
	}

	let proj = proj("cpsparse");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	engine.up(&file).await.unwrap();

	let result = engine
		.cp(&file, payload.to_str().unwrap(), "web:/tmp")
		.await;
	// Both files, and the length of the sparse one: an upload that arrived
	// truncated would pass a check that only asked whether the call returned Ok.
	let out = engine
		.test_exec_capture(
			&format!("{proj}-web-1"),
			vec![
				"sh".into(),
				"-c".into(),
				"cat /tmp/payload/plain.txt && stat -c %s /tmp/payload/holey.bin".into(),
			],
		)
		.await;
	engine.down(&file).await.unwrap();

	// The bytes in the container are still this test's assertion, because this
	// test is about sparse files, not about whether `cp` reports a landed
	// directory copy as success; that lives in
	// `engine_cp_reports_a_directory_copy_as_landed`. If the copy did not land,
	// the error is printed below, so a real failure still says what happened.
	let out = out.unwrap_or_default();
	assert!(
		out.contains("beside-the-hole"),
		"the ordinary file beside the sparse one must arrive too, got {out:?}; \
		 cp returned {result:?}"
	);
	assert!(
		out.contains(&(hole + 4).to_string()),
		"the sparse file must arrive at its full length, got {out:?}; \
		 cp returned {result:?}"
	);
	assert!(
		result.is_ok(),
		"a directory copy of the payload must report Ok on Podman 6 as on Podman 5; \
		 got {result:?}"
	);
}

/// One file with holes, copied to a live container.
///
/// The directory case above is about the bytes; what `cp` returns for a
/// directory is held by `engine_cp_reports_a_directory_copy_as_landed`. A
/// single file is verified by its size, so this one holds `cp` to reporting success as well as to
/// delivering the bytes, and keeps the sparse fix covered on the strict path
/// too. #1775.
#[cfg(all(unix, feature = "test-helpers"))]
#[tokio::test]
async fn engine_cp_uploads_a_sparse_file() {
	use std::io::{Seek, SeekFrom, Write};
	use std::os::unix::fs::MetadataExt;

	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let hole: u64 = 1 << 20;
	let holey = dir.path().join("holey.bin");
	let mut f = fs::File::create(&holey).unwrap();
	f.set_len(hole).unwrap();
	// `set_len` leaves the cursor at 0, so the seek is what puts the tail at the
	// end rather than at the start.
	f.seek(SeekFrom::Start(hole)).unwrap();
	f.write_all(b"tail").unwrap();
	f.sync_all().unwrap();
	drop(f);
	let meta = fs::metadata(&holey).unwrap();
	if meta.blocks() * 512 >= meta.size() {
		eprintln!("cp sparse file: no hole on this filesystem, nothing measured");
		return;
	}

	let proj = proj("cpsparsef");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	engine.up(&file).await.unwrap();

	let result = engine
		.cp(&file, holey.to_str().unwrap(), "web:/tmp/holey.bin")
		.await;
	// The length, not the presence: an upload that arrived truncated would pass
	// a check that only asked whether the file exists.
	let out = engine
		.test_exec_capture(
			&format!("{proj}-web-1"),
			vec!["sh".into(), "-c".into(), "stat -c %s /tmp/holey.bin".into()],
		)
		.await;
	engine.down(&file).await.unwrap();

	result.unwrap_or_else(|e| panic!("cp of a sparse file must upload; podman said: {e}"));
	let out = out.unwrap_or_default();
	assert_eq!(
		out.trim(),
		(hole + 4).to_string(),
		"the sparse file must arrive at its full length"
	);
}
