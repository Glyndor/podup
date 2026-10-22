use super::*;

/// The list comes out of the archive `cp` really builds, with the sizes the
/// files really have. Sorted, because the packer walks in directory order and
/// that is the filesystem's business.
#[test]
fn the_expectation_is_every_file_and_directory_the_packer_wrote() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir_all(payload.join("sub")).unwrap();
	std::fs::create_dir(payload.join("empty")).unwrap();
	std::fs::write(payload.join("plain.txt"), b"beside-the-rest").unwrap();
	std::fs::write(payload.join("sub/inner.bin"), vec![7u8; 1234]).unwrap();
	std::fs::write(payload.join("sub/nothing"), b"").unwrap();

	let tar = pack_to_vec(&payload, false, None, false);

	assert_eq!(
		sorted(sent_entries(&tar).unwrap()),
		vec![
			("payload".to_string(), SentKind::Dir),
			("payload/empty".to_string(), SentKind::Dir),
			("payload/plain.txt".to_string(), SentKind::File(15)),
			("payload/sub".to_string(), SentKind::Dir),
			("payload/sub/inner.bin".to_string(), SentKind::File(1234)),
			("payload/sub/nothing".to_string(), SentKind::File(0)),
		]
	);

	// Renamed on the way in: the destination is asked for the new name.
	let renamed = pack_to_vec(&payload, false, Some("other"), false);
	let paths: Vec<String> = sorted(sent_entries(&renamed).unwrap())
		.into_iter()
		.map(|(path, _)| path)
		.collect();
	assert_eq!(paths[0], "other");
	assert!(
		paths
			.iter()
			.all(|p| p == "other" || p.starts_with("other/")),
		"got {paths:?}"
	);
}

/// A symbolic link is sent and is asked about: the stat endpoint answers 404
/// for a dangling one with the link's own stat in the header, and the link is
/// confirmed there.
#[cfg(unix)]
#[test]
fn a_symlink_is_asked_about_and_confirmed() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir(&payload).unwrap();
	std::fs::write(payload.join("real.txt"), b"x").unwrap();
	std::os::unix::fs::symlink("nowhere", payload.join("dangling")).unwrap();

	let tar = pack_to_vec(&payload, false, None, false);

	assert_eq!(
		sorted(sent_entries(&tar).unwrap()),
		vec![
			("payload".to_string(), SentKind::Dir),
			(
				"payload/dangling".to_string(),
				SentKind::Link("nowhere".to_string()),
			),
			("payload/real.txt".to_string(), SentKind::File(1)),
		]
	);
}

/// The shape a real tree has: two files, a nested directory, a symlink
/// and an empty file, all in one tree. The whole list is asserted, so a
/// missing or extra entry fails the test. The symlink is in the list, because
/// the stat endpoint answers 404 with a link stat in the header and the link is
/// confirmed there.
#[cfg(unix)]
#[test]
fn two_files_a_directory_a_symlink_and_an_empty_file() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir_all(&payload).unwrap();
	std::fs::create_dir(payload.join("nested")).unwrap();
	std::fs::write(payload.join("plain.txt"), b"hi").unwrap();
	std::fs::write(payload.join("nothing.txt"), b"").unwrap();
	std::os::unix::fs::symlink("nowhere", payload.join("link")).unwrap();

	let tar = pack_to_vec(&payload, false, None, false);

	assert_eq!(
		sorted(sent_entries(&tar).unwrap()),
		vec![
			("payload".to_string(), SentKind::Dir),
			(
				"payload/link".to_string(),
				SentKind::Link("nowhere".to_string()),
			),
			("payload/nested".to_string(), SentKind::Dir),
			("payload/nothing.txt".to_string(), SentKind::File(0)),
			("payload/plain.txt".to_string(), SentKind::File(2)),
		],
		"the symlink is in the list alongside the rest"
	);
}

/// A tar whose only entry is a symlink yields one `Link` entry: the stat
/// endpoint answers 404 with a link stat in the header, so the link is
/// confirmed there. The `Link` carries the target the tar header spells,
/// read out of the GNU `linkname` field.
#[cfg(unix)]
#[test]
fn a_tar_with_only_a_symlink_yields_one_link_entry() {
	let dir = tempfile::tempdir().unwrap();
	let link = dir.path().join("dangling");
	std::os::unix::fs::symlink("nowhere", &link).unwrap();

	let tar = pack_to_vec(&link, false, None, false);

	assert_eq!(
		sent_entries(&tar).unwrap(),
		vec![SentEntry {
			path: "dangling".to_string(),
			kind: SentKind::Link("nowhere".to_string()),
		}],
		"only the symlink yields exactly one Link entry"
	);
}

/// A regular file whose name has byte `0xFF` makes `sent_entries` return an
/// error. Without this check the path is rewritten to U+FFFD before the stat,
/// and an existing file literally named with U+FFFD could satisfy the check.
#[cfg(unix)]
#[test]
fn a_path_that_is_not_utf_8_is_an_error() {
	use std::ffi::OsStr;
	use std::os::unix::ffi::OsStrExt;

	let mut builder = tar::Builder::new(Vec::new());
	let mut header = tar::Header::new_gnu();
	header.set_size(0);
	header.set_mode(0o644);
	header.set_entry_type(tar::EntryType::Regular);
	let bad = OsStr::from_bytes(b"bad-\xff-name");
	header.set_path(bad).unwrap();
	header.set_cksum();
	builder.append(&header, std::io::empty()).unwrap();
	let bytes = builder.into_inner().unwrap();

	assert!(
		sent_entries(&bytes).is_err(),
		"a non-UTF-8 path must make sent_entries error, not silently rewrite"
	);
}

/// A FIFO entry in the tar makes `sent_entries` return an error naming the
/// type. The destination cannot be asked about a FIFO through the archive
/// stat, so an archive that hid a FIFO behind a verifiable directory would
/// otherwise be confirmed on the directory alone.
#[cfg(unix)]
#[test]
fn a_fifo_entry_is_an_error() {
	let mut builder = tar::Builder::new(Vec::new());
	let mut header = tar::Header::new_gnu();
	header.set_size(0);
	header.set_mode(0o644);
	header.set_entry_type(tar::EntryType::Fifo);
	header.set_path("pipe").unwrap();
	header.set_cksum();
	builder.append(&header, std::io::empty()).unwrap();
	let bytes = builder.into_inner().unwrap();

	let err = sent_entries(&bytes)
		.expect_err("a FIFO entry must make sent_entries error, not silently skip");
	let msg = err.to_string();
	assert!(
		msg.contains("Fifo"),
		"the error must name the entry type, got: {msg}"
	);
}

/// A hard-link entry in the tar makes `sent_entries` return an error naming the
/// type. Hard links survive a tar round-trip but the destination cannot be
/// asked about one through the archive stat, so an archive that hid a hard
/// link behind a verifiable directory would otherwise be confirmed on the
/// directory alone.
#[cfg(unix)]
#[test]
fn a_hard_link_entry_is_an_error() {
	let mut builder = tar::Builder::new(Vec::new());
	let mut header = tar::Header::new_gnu();
	header.set_size(0);
	header.set_mode(0o644);
	header.set_entry_type(tar::EntryType::Link);
	header.set_path("dup").unwrap();
	header.set_cksum();
	builder.append(&header, std::io::empty()).unwrap();
	let bytes = builder.into_inner().unwrap();

	let err = sent_entries(&bytes)
		.expect_err("a hard-link entry must make sent_entries error, not silently skip");
	let msg = err.to_string();
	assert!(
		msg.contains("Link"),
		"the error must name the entry type, got: {msg}"
	);
}

/// A tar with only the three kinds the destination can be asked about lists
/// exactly those three. The shape is what `pack_path` produces from a directory
/// holding a file, a subdirectory and a symlink, and the regression net for the
/// filter: any change here means the tree confirmation either misses an entry
/// or asks about something it cannot reach.
#[cfg(unix)]
#[test]
fn a_tar_with_file_directory_and_symlink_lists_just_those_three() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir(&payload).unwrap();
	std::fs::create_dir(payload.join("sub")).unwrap();
	std::fs::write(payload.join("plain.txt"), b"hi").unwrap();
	std::os::unix::fs::symlink("nowhere", payload.join("link")).unwrap();

	let tar = pack_to_vec(&payload, false, None, false);

	assert_eq!(
		sorted(sent_entries(&tar).unwrap()),
		vec![
			("payload".to_string(), SentKind::Dir),
			(
				"payload/link".to_string(),
				SentKind::Link("nowhere".to_string()),
			),
			("payload/plain.txt".to_string(), SentKind::File(2)),
			("payload/sub".to_string(), SentKind::Dir),
		],
		"the three verifiable kinds come through, nothing else"
	);
}

/// `cp . svc:/dir` packs under `.`, so the names arrive as `./x` and the root
/// as `.` itself. Joined to the destination as they are, they would stat
/// `/dir/./x`, and the root would stat the destination a second time.
#[test]
fn a_tree_packed_under_dot_is_asked_about_without_the_dot() {
	let mut tar = crate::engine::context::tar_stream::builder(Vec::new());
	let mut dir_header = tar::Header::new_gnu();
	dir_header.set_entry_type(tar::EntryType::Directory);
	dir_header.set_size(0);
	dir_header.set_mode(0o755);
	for name in ["./", "./sub/"] {
		let mut header = dir_header.clone();
		tar.append_data(&mut header, name, std::io::empty())
			.unwrap();
	}
	let mut file_header = tar::Header::new_gnu();
	file_header.set_size(3);
	file_header.set_mode(0o644);
	tar.append_data(&mut file_header, "./sub/f.txt", &b"abc"[..])
		.unwrap();
	let bytes = tar.into_inner().unwrap();

	assert_eq!(
		sorted(sent_entries(&bytes).unwrap()),
		vec![
			("sub".to_string(), SentKind::Dir),
			("sub/f.txt".to_string(), SentKind::File(3)),
		]
	);
}

/// Bytes that are not an archive are an error, which the caller reads as
/// "nothing confirmed". An empty list would say the same thing by accident.
#[test]
fn bytes_that_are_not_an_archive_are_an_error() {
	assert!(sent_entries(b"not a gzipped tar").is_err());
}
