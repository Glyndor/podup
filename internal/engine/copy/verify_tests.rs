use flate2::write::GzEncoder;
use flate2::Compression;

use super::super::archive::pack_path;
use super::{entry_landed, sent_entries, SentEntry, SentKind};
use crate::libpod::client::PathStat;

fn sorted(mut sent: Vec<SentEntry>) -> Vec<(String, SentKind)> {
	sent.sort_by(|a, b| a.path.cmp(&b.path));
	sent.into_iter().map(|e| (e.path, e.kind)).collect()
}

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

	let tar = pack_path(&payload, false, None).unwrap();

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
	let renamed = pack_path(&payload, false, Some("other")).unwrap();
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

/// A link is sent and is not asked about: the stat endpoint answers 404 for a
/// dangling one that is there, and a landed copy must not fail on that.
#[cfg(unix)]
#[test]
fn a_symlink_is_sent_but_not_asked_about() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir(&payload).unwrap();
	std::fs::write(payload.join("real.txt"), b"x").unwrap();
	std::os::unix::fs::symlink("nowhere", payload.join("dangling")).unwrap();

	let tar = pack_path(&payload, false, None).unwrap();

	assert_eq!(
		sorted(sent_entries(&tar).unwrap()),
		vec![
			("payload".to_string(), SentKind::Dir),
			("payload/real.txt".to_string(), SentKind::File(1)),
		]
	);
}

/// The shape the brief calls out: two files, a nested directory, a symlink
/// and an empty file, all in one tree. The whole list is asserted, so a
/// missing or extra entry fails the test. The symlink is filtered, because a
/// dangling one answers 404 on the stat endpoint.
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

	let tar = pack_path(&payload, false, None).unwrap();

	assert_eq!(
		sorted(sent_entries(&tar).unwrap()),
		vec![
			("payload".to_string(), SentKind::Dir),
			("payload/nested".to_string(), SentKind::Dir),
			("payload/nothing.txt".to_string(), SentKind::File(0)),
			("payload/plain.txt".to_string(), SentKind::File(2)),
		],
		"only the symlink is filtered; the rest is whatever was packed"
	);
}

/// A tar whose only entry is a symlink has nothing the destination can be asked
/// about: the entry is filtered for the same reason a symlink in a larger tree
/// is, and an archive that ended up holding just the link yields an empty
/// expectation.
#[cfg(unix)]
#[test]
fn a_tar_with_only_a_symlink_yields_an_empty_expectation() {
	let dir = tempfile::tempdir().unwrap();
	let link = dir.path().join("dangling");
	std::os::unix::fs::symlink("nowhere", &link).unwrap();

	let tar = pack_path(&link, false, None).unwrap();

	assert!(
		sent_entries(&tar).unwrap().is_empty(),
		"a lone symlink is the only kind of entry that produces no expectation; got \
		 a non-empty list"
	);
}

/// `cp . svc:/dir` packs under `.`, so the names arrive as `./x` and the root
/// as `.` itself. Joined to the destination as they are, they would stat
/// `/dir/./x`, and the root would stat the destination a second time.
#[test]
fn a_tree_packed_under_dot_is_asked_about_without_the_dot() {
	let mut tar =
		crate::engine::tar_stream::builder(GzEncoder::new(Vec::new(), Compression::default()));
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
	let gz = tar.into_inner().unwrap().finish().unwrap();

	assert_eq!(
		sorted(sent_entries(&gz).unwrap()),
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

// Modes as Podman 5.7.0 reported them on 2026-09-18: 0644 file, 0755 directory.
const FILE_MODE: u64 = 420;
const DIR_MODE: u64 = 2_147_484_141;

fn stat(size: u64, mode: u64) -> PathStat {
	PathStat {
		size,
		mode,
		..PathStat::default()
	}
}

#[test]
fn a_file_landed_when_it_is_a_file_of_the_size_sent() {
	let sent = SentKind::File(4096);
	assert!(entry_landed(sent, Some(&stat(4096, FILE_MODE))));
	assert!(!entry_landed(sent, Some(&stat(4095, FILE_MODE))));
	assert!(!entry_landed(sent, None));
	// A directory stats at 4096 too. Same number, not the file.
	assert!(!entry_landed(sent, Some(&stat(4096, DIR_MODE))));
	// An empty file is a size like any other.
	assert!(entry_landed(SentKind::File(0), Some(&stat(0, FILE_MODE))));
}

#[test]
fn a_directory_landed_when_a_directory_is_there() {
	assert!(entry_landed(SentKind::Dir, Some(&stat(4096, DIR_MODE))));
	assert!(entry_landed(SentKind::Dir, Some(&stat(0, DIR_MODE))));
	assert!(!entry_landed(SentKind::Dir, Some(&stat(4096, FILE_MODE))));
	assert!(!entry_landed(SentKind::Dir, None));
}
