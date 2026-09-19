//! `cp` flag-parity integration tests (split for the source line limit).
use super::*;

#[tokio::test]
async fn engine_cp_index_out_of_range_errors() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let local = dir.path().join("f.txt");
	fs::write(&local, b"x").unwrap();

	let proj = proj("cpidx");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str("services:\n  web:\n    image: alpine:latest\n").unwrap();

	// Target replica 9 of a single-replica service: must error rather than
	// silently fall back to the first container. The replica-resolution fix
	// reports this as a typed `ReplicaIndex` (named service, 1-based index)
	// instead of a generic `ServiceNotFound`, so the user sees exactly which
	// index is out of range.
	let result = engine
		.cp_with_options(
			&file,
			local.to_str().unwrap(),
			"web:/tmp",
			podup::CpOptions::new(Some(9), false, false),
		)
		.await;
	assert!(
		matches!(
			result,
			Err(podup::ComposeError::ReplicaIndex { ref service, index: 9 }) if service == "web"
		),
		"out-of-range --index must error with ReplicaIndex, got {result:?}"
	);
}

#[cfg(all(unix, feature = "test-helpers"))]
#[tokio::test]
async fn engine_cp_follow_link_uploads_target_contents() {
	use std::os::unix::fs::symlink;
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("target.txt");
	fs::write(&target, b"linked-content").unwrap();
	let link = dir.path().join("link.txt");
	symlink(&target, &link).unwrap();

	let proj = proj("cplnk");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	engine.up(&file).await.unwrap();

	// -L follows the host symlink, so the container receives the target's bytes
	// (a regular file), not a dangling link.
	let result = engine
		.cp_with_options(
			&file,
			link.to_str().unwrap(),
			"web:/tmp",
			podup::CpOptions::new(None, true, false),
		)
		.await;
	let out = engine
		.test_exec_capture(
			&format!("{proj}-web-1"),
			vec!["cat".into(), "/tmp/link.txt".into()],
		)
		.await;
	engine.down(&file).await.unwrap();
	result.unwrap();
	assert!(
		out.unwrap_or_default().contains("linked-content"),
		"-L must upload the symlink target's contents"
	);
}

#[cfg(all(unix, feature = "test-helpers"))]
#[tokio::test]
async fn engine_cp_to_container_renames_a_single_file() {
	// `cp host-file svc:/tmp/newname.txt` must create a FILE named newname.txt,
	// not a directory holding the source, matching `docker cp` rename-on-copy.
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let src = dir.path().join("src.txt");
	fs::write(&src, b"renamed-content").unwrap();

	let proj = proj("cpren");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	engine.up(&file).await.unwrap();

	let result = engine
		.cp(&file, src.to_str().unwrap(), "web:/tmp/renamed.txt")
		.await;
	// `test -f` succeeds only for a regular file; a directory (the old bug) fails it.
	let out = engine
		.test_exec_capture(
			&format!("{proj}-web-1"),
			vec![
				"sh".into(),
				"-c".into(),
				"test -f /tmp/renamed.txt && cat /tmp/renamed.txt".into(),
			],
		)
		.await;
	engine.down(&file).await.unwrap();
	result.unwrap();
	assert!(
		out.unwrap_or_default().contains("renamed-content"),
		"cp to a new name must create a file with the source's content, not a directory"
	);
}

/// I added this test because before #1777 was fixed a directory copy against
/// Podman 6 was reported as failed even when it landed on disk, which is why
/// the neighbour below could not assert on `cp`'s return value. I hold `cp`
/// to both halves here: the bytes arrive, and `cp` says so, on Podman 6 as on
/// Podman 5.
///
/// The payload holds a relative symlink (`link -> a.txt`) and a dangling one
/// (`dangling -> nowhere`), so the live proof that the link confirmation works
/// on the Podman 6 path is the `readlink` line below. A confirmation that
/// dropped the link would either fail outright on a Podman 6 that cannot
/// stat it or land but report `Err`, and `readlink` is the observable that
/// turns the difference into a measurable byte.
#[cfg(all(unix, feature = "test-helpers"))]
#[tokio::test]
async fn engine_cp_reports_a_directory_copy_as_landed() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	fs::create_dir(&payload).unwrap();
	fs::write(payload.join("a.txt"), b"first").unwrap();
	fs::write(payload.join("empty"), b"").unwrap();
	fs::create_dir(payload.join("nested")).unwrap();
	fs::write(payload.join("nested").join("b.txt"), b"second").unwrap();
	std::os::unix::fs::symlink("a.txt", payload.join("link")).unwrap();
	std::os::unix::fs::symlink("nowhere", payload.join("dangling")).unwrap();

	let proj = proj("cpdirland");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	engine.up(&file).await.unwrap();

	let result = engine
		.cp(&file, payload.to_str().unwrap(), "web:/tmp")
		.await;
	let out = engine
		.test_exec_capture(
			&format!("{proj}-web-1"),
			vec![
				"sh".into(),
				"-c".into(),
				"cat /tmp/payload/a.txt /tmp/payload/nested/b.txt && stat -c %s /tmp/payload/empty"
					.into(),
			],
		)
		.await;
	let readlink_out = engine
		.test_exec_capture(
			&format!("{proj}-web-1"),
			vec![
				"sh".into(),
				"-c".into(),
				"readlink /tmp/payload/link && readlink /tmp/payload/dangling".into(),
			],
		)
		.await;
	engine.down(&file).await.unwrap();

	let out = out.unwrap_or_default();
	assert!(
		out.contains("first"),
		"a.txt must arrive at /tmp/payload/a.txt, got {out:?}; cp returned {result:?}"
	);
	assert!(
		out.contains("second"),
		"nested/b.txt must arrive at /tmp/payload/nested/b.txt, got {out:?}; \
		 cp returned {result:?}"
	);
	assert!(
		out.contains("0\n"),
		"empty file must arrive at /tmp/payload/empty with size 0, got {out:?}; \
		 cp returned {result:?}"
	);
	let readlink_out = readlink_out.unwrap_or_default();
	assert!(
		readlink_out.contains("a.txt"),
		"relative symlink must arrive at /tmp/payload/link pointing at a.txt, got {readlink_out:?}; \
		 cp returned {result:?}"
	);
	assert!(
		readlink_out.contains("nowhere"),
		"dangling symlink must arrive at /tmp/payload/dangling pointing at nowhere, got {readlink_out:?}; \
		 cp returned {result:?}"
	);
	assert!(
		result.is_ok(),
		"a directory copy that landed (the bytes above prove it) must report \
		 Ok on Podman 6 as on Podman 5; got {result:?}"
	);
}

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
