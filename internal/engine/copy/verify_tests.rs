use flate2::write::GzEncoder;
use flate2::Compression;

use super::super::archive::pack_path;
use super::{entry_landed, sent_entries, LinkCheck, SentEntry, SentKind};
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

	let tar = pack_path(&payload, false, None).unwrap();

	assert_eq!(
		sorted(sent_entries(&tar).unwrap()),
		vec![
			("payload".to_string(), SentKind::Dir),
			(
				"payload/dangling".to_string(),
				SentKind::Link("nowhere".into())
			),
			("payload/real.txt".to_string(), SentKind::File(1)),
		]
	);
}

/// The shape the brief calls out: two files, a nested directory, a symlink
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

	let tar = pack_path(&payload, false, None).unwrap();

	assert_eq!(
		sorted(sent_entries(&tar).unwrap()),
		vec![
			("payload".to_string(), SentKind::Dir),
			("payload/link".to_string(), SentKind::Link("nowhere".into())),
			("payload/nested".to_string(), SentKind::Dir),
			("payload/nothing.txt".to_string(), SentKind::File(0)),
			("payload/plain.txt".to_string(), SentKind::File(2)),
		],
		"the symlink is in the list alongside the rest"
	);
}

/// A tar whose only entry is a symlink yields one `Link` entry: the stat
/// endpoint answers 404 with a link stat in the header, so the link is
/// confirmed there.
#[cfg(unix)]
#[test]
fn a_tar_with_only_a_symlink_yields_one_link_entry() {
	let dir = tempfile::tempdir().unwrap();
	let link = dir.path().join("dangling");
	std::os::unix::fs::symlink("nowhere", &link).unwrap();

	let tar = pack_path(&link, false, None).unwrap();

	assert_eq!(
		sent_entries(&tar).unwrap(),
		vec![SentEntry {
			path: "dangling".to_string(),
			kind: SentKind::Link("nowhere".into()),
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
	use std::io::Write;
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

	let gz = {
		let mut enc = GzEncoder::new(Vec::new(), Compression::default());
		enc.write_all(&bytes).unwrap();
		enc.finish().unwrap()
	};

	assert!(
		sent_entries(&gz).is_err(),
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
	use std::io::Write;

	let mut builder = tar::Builder::new(Vec::new());
	let mut header = tar::Header::new_gnu();
	header.set_size(0);
	header.set_mode(0o644);
	header.set_entry_type(tar::EntryType::Fifo);
	header.set_path("pipe").unwrap();
	header.set_cksum();
	builder.append(&header, std::io::empty()).unwrap();
	let bytes = builder.into_inner().unwrap();

	let gz = {
		let mut enc = GzEncoder::new(Vec::new(), Compression::default());
		enc.write_all(&bytes).unwrap();
		enc.finish().unwrap()
	};

	let err = sent_entries(&gz)
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
	use std::io::Write;

	let mut builder = tar::Builder::new(Vec::new());
	let mut header = tar::Header::new_gnu();
	header.set_size(0);
	header.set_mode(0o644);
	header.set_entry_type(tar::EntryType::Link);
	header.set_path("dup").unwrap();
	header.set_cksum();
	builder.append(&header, std::io::empty()).unwrap();
	let bytes = builder.into_inner().unwrap();

	let gz = {
		let mut enc = GzEncoder::new(Vec::new(), Compression::default());
		enc.write_all(&bytes).unwrap();
		enc.finish().unwrap()
	};

	let err = sent_entries(&gz)
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
	use super::super::archive::pack_path;

	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir(&payload).unwrap();
	std::fs::create_dir(payload.join("sub")).unwrap();
	std::fs::write(payload.join("plain.txt"), b"hi").unwrap();
	std::os::unix::fs::symlink("nowhere", payload.join("link")).unwrap();

	let tar = pack_path(&payload, false, None).unwrap();

	assert_eq!(
		sorted(sent_entries(&tar).unwrap()),
		vec![
			("payload".to_string(), SentKind::Dir),
			("payload/link".to_string(), SentKind::Link("nowhere".into())),
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
/// 0o644 plus `os.ModeNamedPipe` (1<<25): what a FIFO at the destination looks
/// like in the stat libpod reports. The regular-file check rejects it, because
/// an empty file uploaded over an existing FIFO would otherwise be confirmed
/// by the unchanged pipe (size 0, not a directory).
const FIFO_MODE: u64 = (1 << 25) | 0o644;
/// 0o777 plus `os.ModeSymlink` (1<<27): what a symbolic link at the
/// destination looks like in the stat libpod reports.
const LINK_MODE: u64 = (1 << 27) | 0o777;

fn stat(size: u64, mode: u64) -> PathStat {
	PathStat {
		size,
		mode,
		..PathStat::default()
	}
}

/// A link stat with the given target. `None` for the target leaves
/// `link_target` unset on the `PathStat`, which is what an older runtime
/// (or a stat that never carried `linkTarget`) reports.
fn link_stat(target: Option<&str>) -> PathStat {
	PathStat {
		size: 7,
		mode: LINK_MODE,
		link_target: target.map(str::to_string),
		..PathStat::default()
	}
}

/// Planted-stat matrix for `entry_landed`. The cases the brief names, each in
/// its own assertion so a failure points at the row that regressed.
#[test]
fn entry_landed_with_planted_stats() {
	const DIR: &str = "/tmp";

	// `File(0)` against a FIFO is not a file, so the unchanged pipe cannot
	// confirm a zero-byte upload. This is the regular-file false positive the
	// `MODE_TYPE` mask closes.
	assert_eq!(
		entry_landed(&SentKind::File(0), Some(&stat(0, FIFO_MODE)), DIR),
		LinkCheck::Refused,
	);

	// `File(0)` against a regular file at size 0 is the regular-file landed
	// shape; the assertion is the regression net for the mask.
	assert_eq!(
		entry_landed(&SentKind::File(0), Some(&stat(0, FILE_MODE)), DIR),
		LinkCheck::Confirmed,
	);

	// `File(4096)` against a directory is not a file either: a directory stats
	// at 4096 on most filesystems, the same size as the file.
	assert_eq!(
		entry_landed(&SentKind::File(4096), Some(&stat(4096, DIR_MODE)), DIR,),
		LinkCheck::Refused,
	);

	// `Link("a.txt")` against a symlink whose `linkTarget` is the same string
	// is the link-confirmation shape: the symlink bit AND the target match.
	assert_eq!(
		entry_landed(
			&SentKind::Link("a.txt".into()),
			Some(&link_stat(Some("a.txt"))),
			DIR,
		),
		LinkCheck::Confirmed,
	);

	// `Link("a.txt")` against a regular file is not a link: a regular file
	// cannot satisfy the link confirmation, even at the right mode bits.
	assert_eq!(
		entry_landed(
			&SentKind::Link("a.txt".into()),
			Some(&stat(7, FILE_MODE)),
			DIR
		),
		LinkCheck::Refused,
	);
}

/// Planted-stat matrix for the link-target half of the confirmation.
///
/// The brief names four cases, each in its own assertion so a failure points
/// at the row that regressed:
///
/// - a matching target confirms (the strong path);
/// - a DIFFERENT target does not confirm (the strong path rejected the
///   upload);
/// - a stat with NO target confirms on the symlink bit alone (the documented
///   residual on a runtime that does not report link targets);
/// - a stat whose `link_target` is the empty string also confirms on the
///   symlink bit alone (a symlink always points at something, so an empty
///   string is not a valid symlink target);
/// - a stat with no target AND no symlink bit does not confirm (the entry is
///   not what was sent).
///
/// The previous test asserted that a stat without a target is unconfirmed.
/// That assertion is what failed on Podman 6: the copy landed, the runtime
/// answered the symlink stat without `linkTarget`, and the strict equality
/// treated the entry as unconfirmed. The brief mandates the fallback on the
/// symlink bit, with a `tracing::warn!` at the call site, and the new
/// assertion reflects that.
#[test]
fn a_link_is_landed_only_when_its_target_matches_what_was_sent() {
	const DIR: &str = "/tmp";
	let sent = SentKind::Link("a.txt".into());

	// Target matches: this is the link-confirmation shape.
	assert_eq!(
		entry_landed(&sent, Some(&link_stat(Some("a.txt"))), DIR),
		LinkCheck::Confirmed,
		"a link whose target equals what was sent must be confirmed"
	);

	// Target differs: the destination's `linkTarget` says `elsewhere`, the
	// archive carried `a.txt`. The strong path rejects it; the symlink bit
	// alone is not enough.
	assert_eq!(
		entry_landed(&sent, Some(&link_stat(Some("elsewhere"))), DIR),
		LinkCheck::Refused,
		"a link pointing at a different target than what was sent must not be confirmed"
	);

	// Target absent, symlink bit set: the stat carries no `linkTarget`, so
	// the destination cannot answer the question any stronger than the bit.
	// The fallback confirms here, because refusing a copy that landed would
	// re-introduce the #1777 shape this module exists to close. The call
	// site emits a `warn!` so a CI log carries the reason; the residual is
	// that a pre-existing link pointing elsewhere still passes here. That is
	// the documented price of not failing a landed copy.
	assert_eq!(
		entry_landed(&sent, Some(&link_stat(None)), DIR),
		LinkCheck::Fallback,
		"a stat without linkTarget falls back to the symlink bit, with a warn at the call site"
	);

	// Target is the empty string: a symlink always points at something, so
	// a runtime that reports `""` for a symlink has not answered the
	// question, exactly as one that omits the field has not. Same branch
	// as the absent case above. Treating the empty string as a target to
	// compare against would have refused the entry, which is the #1777
	// shape this module was written to close.
	assert_eq!(
		entry_landed(&sent, Some(&link_stat(Some(""))), DIR),
		LinkCheck::Fallback,
		"an empty link_target falls back to the symlink bit, the same as an absent one"
	);

	// Target absent AND no symlink bit: the entry is not what was sent.
	// The fallback cannot apply because there is no symlink bit to fall back
	// to. The entry is refused.
	assert_eq!(
		entry_landed(
			&sent,
			Some(&PathStat {
				size: 7,
				mode: FILE_MODE,
				link_target: None,
				..PathStat::default()
			}),
			DIR,
		),
		LinkCheck::Refused,
		"a stat with no target and no symlink bit is not the link that was sent"
	);

	// No stat at all: the destination returned `None`, which is the path
	// the stat endpoint takes for a regular file or directory that is not
	// present. The link is absent.
	assert_eq!(
		entry_landed(&sent, None, DIR),
		LinkCheck::Absent,
		"an absent stat does not confirm a link"
	);
}

/// An absolute link target the runtime reports is the same link as a relative
/// archive target when the absolute one, joined to the destination directory,
/// resolves to the relative one. Podman 6 has been seen to expand relative
/// linknames against the destination and report the absolute result; the
/// confirmation here answers that without losing the strict equality for the
/// case where the two strings disagree.
#[test]
fn an_absolute_runtime_target_resolves_to_the_relative_archive_target() {
	const DIR: &str = "/tmp";
	let sent = SentKind::Link("a.txt".into());

	assert_eq!(
		entry_landed(&sent, Some(&link_stat(Some("/tmp/a.txt"))), DIR),
		LinkCheck::Confirmed,
		"an absolute target that joins with the destination directory to the relative target confirms",
	);

	// An absolute target that does not resolve under the destination cannot
	// be related to the archive's relative target and is refused on the strong
	// path, same as a plainly different target.
	assert_eq!(
		entry_landed(&sent, Some(&link_stat(Some("/elsewhere/a.txt"))), DIR),
		LinkCheck::Refused,
		"an absolute target outside the destination directory is not the link that was sent",
	);

	// An absolute target with multiple segments after the destination
	// directory (a deeper path) is not the link the archive carried either.
	assert_eq!(
		entry_landed(&sent, Some(&link_stat(Some("/tmp/sub/a.txt"))), DIR),
		LinkCheck::Refused,
		"an absolute target that descends below a single segment is not the link that was sent",
	);
}

#[test]
fn a_file_landed_when_it_is_a_file_of_the_size_sent() {
	const DIR: &str = "/tmp";
	let sent = SentKind::File(4096);
	assert_eq!(
		entry_landed(&sent, Some(&stat(4096, FILE_MODE)), DIR),
		LinkCheck::Confirmed
	);
	assert_eq!(
		entry_landed(&sent, Some(&stat(4095, FILE_MODE)), DIR),
		LinkCheck::Refused
	);
	assert_eq!(entry_landed(&sent, None, DIR), LinkCheck::Absent);
	// A directory stats at 4096 too. Same number, not the file.
	assert_eq!(
		entry_landed(&sent, Some(&stat(4096, DIR_MODE)), DIR),
		LinkCheck::Refused
	);
	// An empty file is a size like any other.
	assert_eq!(
		entry_landed(&SentKind::File(0), Some(&stat(0, FILE_MODE)), DIR),
		LinkCheck::Confirmed
	);
}

#[test]
fn a_directory_landed_when_a_directory_is_there() {
	const DIR: &str = "/tmp";
	assert_eq!(
		entry_landed(&SentKind::Dir, Some(&stat(4096, DIR_MODE)), DIR),
		LinkCheck::Confirmed
	);
	assert_eq!(
		entry_landed(&SentKind::Dir, Some(&stat(0, DIR_MODE)), DIR),
		LinkCheck::Confirmed
	);
	assert_eq!(
		entry_landed(&SentKind::Dir, Some(&stat(4096, FILE_MODE)), DIR),
		LinkCheck::Refused
	);
	assert_eq!(entry_landed(&SentKind::Dir, None, DIR), LinkCheck::Absent);
}
