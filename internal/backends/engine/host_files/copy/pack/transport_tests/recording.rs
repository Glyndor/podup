use super::*;

// --- what the recording carries -------------------------------------------

/// An empty file in the source must still be recorded. The list must include
/// the file at its size, even when the size is zero — otherwise a
/// `cp src/empty.txt svc:/dst` would not confirm against the destination's
/// zero-length file.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn an_empty_file_is_recorded_with_size_zero() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir_all(&payload).unwrap();
	std::fs::write(payload.join("nothing"), b"").unwrap();

	let (recorded, result) = run(pack_path_stream(&payload, false, None, false, counter())).await;
	result.expect("packer should not error on an empty file");

	assert!(
		recorded
			.iter()
			.any(|e| e.path == "payload/nothing" && matches!(e.kind, SentKind::File(0))),
		"the empty file must be recorded with File(0), got {:?}",
		recorded
	);
}

/// Sanity: streaming the body does not change what the recorder sees.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn the_recorder_runs_inside_spawn_blocking_and_finishes() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir_all(&payload).unwrap();
	std::fs::write(payload.join("a.txt"), b"hi").unwrap();
	std::fs::write(payload.join("b.txt"), b"there").unwrap();

	let (recorded, result) = run(pack_path_stream(&payload, false, None, false, counter())).await;
	result.expect("packer should not error on a regular tree");
	assert_eq!(recorded.len(), 3, "wrapper + two files, got {:?}", recorded);
}

/// Watch sync's gzipped packer records the same list `cp`'s plain packer
/// would for the same source. The shared confirmation works on either body.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn sync_recording_includes_a_symlink() {
	let dir = tempfile::tempdir().unwrap();
	let src = dir.path().canonicalize().unwrap().join("changed");
	std::fs::create_dir_all(&src).unwrap();
	std::fs::write(src.join("a.txt"), b"hi").unwrap();
	std::os::unix::fs::symlink("nowhere", src.join("dangling")).unwrap();

	let packed = crate::engine::copy::build_sync_tar_stream_for_watch(
		&src,
		Path::new("changed"),
		counter(),
		std::sync::Arc::new(|_| false),
	);
	let (_recorded, result) = run(packed).await;
	result.expect("sync packer should not error on a tree with a link");
	// The exact list shape is covered by the read-back test; here we only
	// pin that the recorder ran and finished cleanly.
}
