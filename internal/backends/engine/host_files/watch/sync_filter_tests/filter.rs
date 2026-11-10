use super::*;

// --- per-entry filter: root and single files ----------------------------

/// A directory upload with a non-empty entry name whose filter rejects
/// exactly the root name: the root directory header is dropped (the
/// packer still walks through it), its descendants are recorded
/// under their re-rooted paths, and the recorded `SentEntry` list
/// matches the tar contents. The recorder and the archive bytes
/// cannot drift, so a sabotage that drops an entry from the recorder
/// but keeps it in the tar (or the reverse) is caught.
#[test]
fn sync_tar_filter_drops_the_root_directory_keeps_descendants() {
	let dir = tempdir().unwrap();
	let outer = dir.path().join("outer");
	fs::create_dir(&outer).unwrap();
	fs::write(outer.join("a.txt"), b"a").unwrap();
	fs::create_dir(outer.join("sub")).unwrap();
	fs::write(outer.join("sub/b.txt"), b"b").unwrap();

	let skip = |name: &Path| name == Path::new("outer");
	let (bytes, sent) =
		sync_to_gz_with_sent(&outer, Path::new("outer"), &skip).expect("pack succeeds");

	let entries = tar_entries(&bytes);
	assert_eq!(
		entries,
		vec![
			("outer/a.txt".into(), "file"),
			("outer/sub".into(), "dir"),
			("outer/sub/b.txt".into(), "file"),
		],
		"filter on the root must drop the root directory entry and keep the descendants, got {entries:?}"
	);

	let sent_names: Vec<String> = sent.iter().map(|e| e.path().replace('\\', "/")).collect();
	assert_eq!(
		sent_names,
		vec![
			"outer/a.txt".to_string(),
			"outer/sub".to_string(),
			"outer/sub/b.txt".to_string(),
		],
		"recorded SentEntry list must match the tar contents, got {sent_names:?}"
	);
}

/// A single-file upload whose filter rejects its entry name records
/// nothing: the tar is a valid empty gzipped stream and the recorded
/// `SentEntry` list is empty. The verification step would then have
/// nothing to confirm against and the short-circuit in
/// `sync_to_container` would have already returned `Ok(())` before
/// the PUT ran.
#[test]
fn sync_tar_filter_drops_a_single_file() {
	let dir = tempdir().unwrap();
	let file = dir.path().join("solo.txt");
	fs::write(&file, b"only").unwrap();

	let skip = |name: &Path| name == Path::new("solo.txt");
	let (bytes, sent) =
		sync_to_gz_with_sent(&file, Path::new("solo.txt"), &skip).expect("pack succeeds");

	// The tar is still a valid gzipped stream, just with no entries.
	assert_eq!(
		&bytes[..2],
		&[0x1f, 0x8b],
		"tar must still be a valid gzip stream"
	);
	let entries = tar_entries(&bytes);
	assert!(
		entries.is_empty(),
		"the only entry the filter dropped must not appear in the tar, got {entries:?}"
	);
	assert!(
		sent.is_empty(),
		"the only entry the filter dropped must not be recorded, got {sent:?}"
	);
}
