//! Tests for `pack_path`, the unified `cp` packer.
//!
//! These tests drive the production function directly: the unified packer
//! takes a `tar::Builder<W: Write>` and a recorder, so the tests build a
//! builder over a `Vec<u8>` and read the archive back through the same API
//! the streaming path uses. The parity tests in `pack_tests.rs` compare the
//! recorder against `sent_entries` of these bytes; the tests here check the
//! archive shape (`pack_path_entry_names`) and the error categories.

use std::io::Read;
use std::path::Path;

/// Drive `pack_path` into a `Vec<u8>` and return the bytes, mirroring what
/// the streaming path produces on the wire. The unified packer takes a
/// `tar::Builder<W: Write>` and a recorder; the helper builds a builder over
/// a `Vec<u8>` so the test can read the archive back and assert on its
/// shape. The recorder is dropped — the parity check on it lives in
/// `pack_tests.rs`.
fn pack_to_vec(
	src: &Path,
	follow_link: bool,
	name_override: Option<&str>,
	contents: bool,
) -> Vec<u8> {
	let mut buf = Vec::new();
	{
		let mut tar = tar::Builder::new(&mut buf);
		let mut sent = Vec::new();
		super::pack_path(
			src,
			follow_link,
			name_override,
			contents,
			&mut tar,
			&mut sent,
		)
		.expect("pack_path");
	}
	buf
}

/// Drive `pack_path` and return both the bytes and the recorded entry list.
/// Drives the parity test in `pack_tests.rs` (`recorded == sent_entries` of
/// the same bytes), so every test that uses it pins that the recorder and
/// the archive agree.
fn pack_with_recorder(
	src: &Path,
	follow_link: bool,
	name_override: Option<&str>,
	contents: bool,
) -> (Vec<u8>, Vec<super::super::verify::SentEntry>) {
	let mut buf = Vec::new();
	let mut sent = Vec::new();
	{
		let mut tar = tar::Builder::new(&mut buf);
		super::pack_path(
			src,
			follow_link,
			name_override,
			contents,
			&mut tar,
			&mut sent,
		)
		.expect("pack_path");
	}
	(buf, sent)
}

#[test]
fn pack_path_single_file() {
	let dir = tempfile::tempdir().expect("tempdir");
	let file = dir.path().join("data.txt");
	std::fs::write(&file, b"hello").expect("write");
	let bytes = pack_to_vec(&file, false, None, false);
	assert!(!bytes.is_empty());
}

/// The local socket carries plain tar, not a gzip stream. Read the bytes
/// themselves rather than trusting the constructor: a gzip stream starts with
/// the magic header `0x1f 0x8b`, and the tar header is the entry's name as
/// ASCII.
#[test]
fn pack_path_archive_is_not_gzip_compressed() {
	let dir = tempfile::tempdir().expect("tempdir");
	let file = dir.path().join("data.txt");
	std::fs::write(&file, b"hello").expect("write");
	let bytes = pack_to_vec(&file, false, None, false);
	assert!(
		bytes.len() >= 2,
		"archive must have at least two bytes, got {}",
		bytes.len()
	);
	assert_ne!(
		&bytes[..2],
		&[0x1f, 0x8b],
		"archive must not start with the gzip magic header: first bytes are {:02x?}",
		&bytes[..bytes.len().min(8)]
	);
	// And it is a plain tar the destination can extract: one entry, named
	// after the source file.
	let mut archive = tar::Archive::new(bytes.as_slice());
	let names: Vec<String> = archive
		.entries()
		.expect("entries")
		.map(|e| {
			e.expect("entry")
				.path()
				.expect("path")
				.to_string_lossy()
				.into_owned()
		})
		.collect();
	assert_eq!(names, vec!["data.txt".to_string()]);
}

#[test]
fn pack_path_directory() {
	let dir = tempfile::tempdir().expect("tempdir");
	let subdir = dir.path().join("mydir");
	std::fs::create_dir(&subdir).expect("mkdir");
	std::fs::write(subdir.join("a.txt"), b"aaa").expect("write");
	std::fs::write(subdir.join("b.txt"), b"bbb").expect("write");
	let bytes = pack_to_vec(&subdir, false, None, false);
	assert!(!bytes.is_empty());
}

#[test]
fn pack_path_missing_source_is_a_cp_error() {
	// A missing host source on `cp` must read as a cp error, not a build error.
	// `src.is_dir()` returns false for a missing path, so `record_one` reaches
	// `symlink_metadata`, which fails on a missing file; the error mapper
	// (`cp_err`) wraps that as a `ComposeError::Copy`.
	let missing = Path::new("/nonexistent-host-source-xyz");
	let mut tar = tar::Builder::new(Vec::new());
	let mut sent = Vec::new();
	let err = super::pack_path(missing, false, None, false, &mut tar, &mut sent).unwrap_err();
	let msg = err.to_string();
	assert!(msg.contains("cp error"), "wrong category: {msg:?}");
	assert!(
		!msg.contains("build error"),
		"must not be a build error: {msg:?}"
	);
}

/// Read the entry names of the tar built by `pack_path`, which is plain tar
/// since the bytes are handed to a local Unix socket. Sorted because the
/// walker goes in directory order, which is the filesystem's business.
fn pack_path_entry_names(bytes: &[u8]) -> Vec<String> {
	let mut archive = tar::Archive::new(bytes);
	let mut names: Vec<String> = Vec::new();
	for entry in archive.entries().expect("entries") {
		let entry = entry.expect("entry");
		let path = entry.path().expect("path");
		// Skip the top-level wrapper a directory archive adds; we want the
		// inner shape, which is what `cp` actually puts in the container.
		for component in path.components() {
			if let std::path::Component::Normal(p) = component {
				names.push(p.to_string_lossy().into_owned());
			}
		}
	}
	names.sort();
	names
}

/// `cp host/. svc:/X` packs the directory's children at the top of the archive
/// instead of under the directory's name. There is no `payload/` wrapper
/// entry: the names that come back are the directory's children, exactly.
#[test]
fn pack_path_contents_packs_children_at_the_top_with_no_wrapper() {
	let dir = tempfile::tempdir().expect("tempdir");
	let payload = dir.path().join("payload");
	std::fs::create_dir(&payload).expect("mkdir");
	std::fs::write(payload.join("a.txt"), b"aaa").expect("write");
	std::fs::write(payload.join("b.txt"), b"bbb").expect("write");

	let bytes = pack_to_vec(&payload, false, None, true);

	// No wrapper, no `payload/` prefix: every name the archive lists is a
	// child of the source directory.
	let names = pack_path_entry_names(&bytes);
	assert_eq!(
		names,
		vec!["a.txt".to_string(), "b.txt".to_string()],
		"trailing `/.` packs the children at the top, got {names:?}"
	);
	assert!(
		!names.iter().any(|n| n == "payload"),
		"no wrapper entry named after the source dir, got {names:?}"
	);
}

/// Without the `/.` marker the wrapper is back: the same tree is now packed
/// under `payload/...`, the way `cp` has always packed a directory.
#[test]
fn pack_path_no_contents_keeps_the_wrapper() {
	let dir = tempfile::tempdir().expect("tempdir");
	let payload = dir.path().join("payload");
	std::fs::create_dir(&payload).expect("mkdir");
	std::fs::write(payload.join("a.txt"), b"aaa").expect("write");
	std::fs::write(payload.join("b.txt"), b"bbb").expect("write");

	let bytes = pack_to_vec(&payload, false, None, false);

	let names = pack_path_entry_names(&bytes);
	assert!(
		names.contains(&"payload".to_string()),
		"the wrapper entry must be present without the trailing `/.`, got {names:?}"
	);
	assert!(
		names.contains(&"a.txt".to_string()) && names.contains(&"b.txt".to_string()),
		"the children must still be in the archive, got {names:?}"
	);
}

/// `cp single-file/. svc:/X` is an error: the `/.` marker demands a
/// directory. Matches the "not a directory" shape `podman cp` returns.
#[test]
fn pack_path_contents_on_a_single_file_is_an_error() {
	let dir = tempfile::tempdir().expect("tempdir");
	let file = dir.path().join("single.txt");
	std::fs::write(&file, b"hi").expect("write");

	let mut tar = tar::Builder::new(Vec::new());
	let mut sent = Vec::new();
	let err = super::pack_path(&file, false, None, true, &mut tar, &mut sent).unwrap_err();
	let msg = err.to_string();
	assert!(
		msg.contains("not a directory"),
		"single-file source with `/.` must error as not a directory, got {msg:?}"
	);
}

/// The recorded list equals what `sent_entries` reads back from the same
/// bytes. Pack to a `Vec` with the recorder on, then assert
/// `recorded == sent_entries(&vec)`. Pins that the recorder and the archive
/// bytes agree on every entry — a "tar.append_link without recording"
/// shape breaks the parity, and any future drift on the other side breaks
/// this too.
#[cfg(unix)]
#[test]
fn the_recorder_agrees_with_what_sent_entries_reads_back() {
	let dir = tempfile::tempdir().expect("tempdir");
	let payload = dir.path().join("payload");
	std::fs::create_dir_all(payload.join("sub")).unwrap();
	std::fs::create_dir(payload.join("empty")).unwrap();
	std::fs::write(payload.join("plain.txt"), b"beside-the-rest").unwrap();
	std::fs::write(payload.join("sub/inner.bin"), vec![7u8; 1234]).unwrap();
	std::fs::write(payload.join("sub/nothing"), b"").unwrap();
	std::os::unix::fs::symlink("nowhere", payload.join("dangling")).unwrap();

	let (bytes, recorded) = pack_with_recorder(&payload, false, None, false);
	let from_archive = super::super::verify::sent_entries(&bytes).expect("sent_entries");

	let mut a: Vec<(String, String)> = recorded
		.iter()
		.map(|e| (e.path.clone(), format!("{:?}", e.kind)))
		.collect();
	let mut b: Vec<(String, String)> = from_archive
		.iter()
		.map(|e| (e.path.clone(), format!("{:?}", e.kind)))
		.collect();
	a.sort();
	b.sort();
	assert_eq!(
		a, b,
		"the recorder must equal sent_entries of the same bytes"
	);
}

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
		super::super::verify::SentKind::File(payload.len() as u64),
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
		super::super::verify::SentKind::Link(expected_target.clone()),
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
	let mut by_path: std::collections::HashMap<String, super::super::verify::SentKind> = recorded
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
		super::super::verify::SentKind::File(payload_bytes.len() as u64),
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
		super::super::verify::SentKind::Link(_)
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
