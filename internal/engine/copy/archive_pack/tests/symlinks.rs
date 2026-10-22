use super::*;

/// `cp -L` on a symlink to a regular file: the archive holds the target's
/// bytes (a regular entry), not a link, and the recorder holds `File(len)`.
/// The recorder is what the post-PUT confirmation reads, so this is the
/// observable that fails the live test when `follow_link` is dropped.
#[cfg(unix)]
#[test]
fn pack_path_follow_link_on_a_symlink_to_a_file_writes_the_targets_bytes() {
	let dir = tempfile::tempdir().expect("tempdir");
	let target = dir.path().join("target.txt");
	let payload = b"contents-of-the-target-file";
	std::fs::write(&target, payload).expect("write target");
	let link = dir.path().join("link.txt");
	std::os::unix::fs::symlink(&target, &link).expect("symlink");

	let (bytes, recorded) = pack_with_recorder(&link, true, None, false);

	// The recorder must have classified the entry as a regular file of the
	// target's size, not as a link. This is what the live test fails on when
	// `follow_link` is dropped: the recorder says `Link(".../target.txt")`,
	// the destination is asked about a link that never lands, and the
	// confirmation refuses.
	assert_eq!(
		recorded.len(),
		1,
		"a single-link source produces one entry, got {recorded:?}"
	);
	assert_eq!(
		recorded[0].path, "link.txt",
		"the archive path is the source's basename"
	);
	assert_eq!(
		recorded[0].kind,
		super::super::super::verify::SentKind::File(payload.len() as u64),
		"follow_link=true must record the target as a regular file of the target's size"
	);

	// And the archive itself: a plain tar reading back the bytes must show
	// exactly one regular entry carrying the target's content. Read inside
	// the iteration: each entry's data is a `Take` over the archive's
	// underlying reader, so reading after `collect()` would advance past
	// the body and yield the EOF padding instead of the bytes.
	let mut archive = tar::Archive::new(bytes.as_slice());
	let mut entry_count = 0;
	let mut entry_buf = Vec::new();
	let mut entry_size = 0u64;
	let mut entry_kind = tar::EntryType::Regular;
	for entry in archive.entries().expect("entries") {
		let mut entry = entry.expect("entry");
		entry_count += 1;
		entry_kind = entry.header().entry_type();
		entry_size = entry.header().size().expect("size");
		entry.read_to_end(&mut entry_buf).expect("read entry");
	}
	assert_eq!(entry_count, 1, "one entry in the archive");
	assert_eq!(entry_kind, tar::EntryType::Regular);
	assert_eq!(entry_size, payload.len() as u64);
	assert_eq!(
		entry_buf, payload,
		"the archive must hold the target's bytes"
	);
}

/// Same link, `follow_link=false`: the archive holds a symlink entry whose
/// target is the file the link pointed at, and the recorder holds
/// `Link(target)`. Pinned next to the `true` case so the difference between
/// the two is visible in one test file.
#[cfg(unix)]
#[test]
fn pack_path_no_follow_link_on_a_symlink_writes_a_link_entry() {
	let dir = tempfile::tempdir().expect("tempdir");
	let target = dir.path().join("target.txt");
	std::fs::write(&target, b"contents").expect("write target");
	let link = dir.path().join("link.txt");
	std::os::unix::fs::symlink(&target, &link).expect("symlink");

	let (bytes, recorded) = pack_with_recorder(&link, false, None, false);

	assert_eq!(recorded.len(), 1);
	assert_eq!(recorded[0].path, "link.txt");
	let expected_target = target.to_str().expect("utf-8 target path").to_string();
	assert_eq!(
		recorded[0].kind,
		super::super::super::verify::SentKind::Link(expected_target.clone()),
		"follow_link=false records the link itself"
	);

	// Archive shape: one symlink entry, no file body, target matches.
	let mut archive = tar::Archive::new(bytes.as_slice());
	let mut entry_count = 0;
	let mut entry_kind = tar::EntryType::Regular;
	let mut link_name: Option<Vec<u8>> = None;
	for entry in archive.entries().expect("entries") {
		let entry = entry.expect("entry");
		entry_count += 1;
		entry_kind = entry.header().entry_type();
		link_name = entry.link_name_bytes().map(|b| b.into_owned());
	}
	assert_eq!(entry_count, 1);
	assert_eq!(entry_kind, tar::EntryType::Symlink);
	assert_eq!(
		link_name.expect("link name").as_slice(),
		expected_target.as_bytes()
	);
}

/// `cp -L payload/ svc:/X` where `payload/` contains a symlink-to-file:
/// the link inside the tree is replaced by the target's contents as a
/// regular entry at `payload/<link>`, and the recorder holds `File(len)`.
/// The wrapper is still a directory at `payload/`.
#[cfg(unix)]
#[test]
fn pack_path_follow_link_in_a_tree_writes_link_targets_as_files() {
	let dir = tempfile::tempdir().expect("tempdir");
	let payload = dir.path().join("payload");
	std::fs::create_dir(&payload).expect("mkdir");
	let target = dir.path().join("outside.txt");
	let payload_bytes = b"contents-of-the-link-target-inside-a-tree";
	std::fs::write(&target, payload_bytes).expect("write target");
	std::os::unix::fs::symlink(&target, payload.join("link.txt")).expect("symlink");

	let (bytes, recorded) = pack_with_recorder(&payload, true, None, false);

	// The recorder must show the link replaced by a regular file of the
	// target's size, sitting inside the wrapper as `payload/link.txt`.
	let mut by_path: std::collections::HashMap<String, super::super::super::verify::SentKind> =
		recorded
			.iter()
			.map(|e| (e.path.clone(), e.kind.clone()))
			.collect();
	assert!(
		by_path.remove("payload").is_some(),
		"the wrapper entry must be present, got {by_path:?}"
	);
	assert!(
		by_path.remove("payload/link.txt").is_some(),
		"the link inside the tree must be present, got {by_path:?}"
	);
	let link_kind = recorded
		.iter()
		.find(|e| e.path == "payload/link.txt")
		.expect("link entry")
		.kind
		.clone();
	assert_eq!(
		link_kind,
		super::super::super::verify::SentKind::File(payload_bytes.len() as u64),
		"the link inside the tree must be recorded as a regular file of the target's size"
	);

	// Archive shape: a directory at `payload/`, then a regular file at
	// `payload/link.txt` whose body is the target's bytes.
	let mut archive = tar::Archive::new(bytes.as_slice());
	let entries: Vec<_> = archive
		.entries()
		.expect("entries")
		.map(|e| e.expect("entry"))
		.collect();
	let names: Vec<String> = entries
		.iter()
		.map(|e| e.path().expect("path").to_string_lossy().into_owned())
		.collect();
	assert!(names.contains(&"payload".to_string()), "got {names:?}");
	assert!(
		names.contains(&"payload/link.txt".to_string()),
		"got {names:?}"
	);
	// Read inside the iteration: each entry's data is a `Take` over the
	// archive's underlying reader, so reading after `collect()` would
	// advance past the body and yield the EOF padding instead of the bytes.
	let mut link_kind = tar::EntryType::Regular;
	let mut link_size = 0u64;
	let mut link_buf = Vec::new();
	{
		let mut archive = tar::Archive::new(bytes.as_slice());
		for entry in archive.entries().expect("entries") {
			let mut entry = entry.expect("entry");
			if entry.path().expect("path").to_string_lossy() == "payload/link.txt" {
				link_kind = entry.header().entry_type();
				link_size = entry.header().size().expect("size");
				entry.read_to_end(&mut link_buf).expect("read entry");
			}
		}
	}
	assert_eq!(link_kind, tar::EntryType::Regular);
	assert_eq!(link_size, payload_bytes.len() as u64);
	assert_eq!(
		link_buf, payload_bytes,
		"the archive must hold the target's bytes"
	);
}

/// `cp payload/ svc:/X` (no `-L`): the link inside the tree stays a link,
/// the recorder holds `Link(target)`, and the archive holds a symlink entry.
#[cfg(unix)]
#[test]
fn pack_path_no_follow_link_in_a_tree_keeps_links_as_links() {
	let dir = tempfile::tempdir().expect("tempdir");
	let payload = dir.path().join("payload");
	std::fs::create_dir(&payload).expect("mkdir");
	let target = dir.path().join("outside.txt");
	std::fs::write(&target, b"contents").expect("write target");
	std::os::unix::fs::symlink(&target, payload.join("link.txt")).expect("symlink");

	let (bytes, recorded) = pack_with_recorder(&payload, false, None, false);

	// Recorder: the wrapper is a dir, the inside is a link.
	let link_entry = recorded
		.iter()
		.find(|e| e.path == "payload/link.txt")
		.expect("link entry");
	assert!(matches!(
		link_entry.kind,
		super::super::super::verify::SentKind::Link(_)
	));

	// Archive: a symlink entry at `payload/link.txt`. Reading inside the
	// iteration: each entry's data is a `Take` over the archive's underlying
	// reader, so reading after the iteration would advance past the body.
	let mut archive = tar::Archive::new(bytes.as_slice());
	let mut found_kind = tar::EntryType::Regular;
	let mut found = false;
	for entry in archive.entries().expect("entries") {
		let entry = entry.expect("entry");
		if entry.path().expect("path").to_string_lossy() == "payload/link.txt" {
			found_kind = entry.header().entry_type();
			found = true;
		}
	}
	assert!(found, "link.txt must be present in the archive");
	assert_eq!(found_kind, tar::EntryType::Symlink);
}
