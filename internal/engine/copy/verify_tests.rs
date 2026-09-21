use super::super::archive::pack_path;
use super::{
	entry_landed, format_landed_failure, sent_entries, LandedFailure, LinkCheck, SentEntry,
	SentKind,
};
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

	let tar = pack_path(&payload, false, None, false).unwrap();

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
	let renamed = pack_path(&payload, false, Some("other"), false).unwrap();
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

	let tar = pack_path(&payload, false, None, false).unwrap();

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

	let tar = pack_path(&payload, false, None, false).unwrap();

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

	let tar = pack_path(&link, false, None, false).unwrap();

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
	use super::super::archive::pack_path;

	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir(&payload).unwrap();
	std::fs::create_dir(payload.join("sub")).unwrap();
	std::fs::write(payload.join("plain.txt"), b"hi").unwrap();
	std::os::unix::fs::symlink("nowhere", payload.join("link")).unwrap();

	let tar = pack_path(&payload, false, None, false).unwrap();

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
	let mut tar = crate::engine::tar_stream::builder(Vec::new());
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

/// Planted-stat matrix for `entry_landed`. The cases the brief names, each in
/// its own assertion so a failure points at the row that regressed. The link
/// rows use a sent target and a `link_target` whose equality is the only thing
/// that confirms a symlink at the destination; the planted stats here leave
/// `link_target` empty so the link lands on the `Fallback` path (the
/// pre-#1808 type check), and the dedicated link tests below cover the
/// target-driven confirmation.
#[test]
fn entry_landed_with_planted_stats() {
	// `File(0)` against a FIFO is not a file, so the unchanged pipe cannot
	// confirm a zero-byte upload. This is the regular-file false positive the
	// `MODE_TYPE` mask closes.
	assert_eq!(
		entry_landed(SentKind::File(0), "/tmp/x", Some(&stat(0, FIFO_MODE))),
		LinkCheck::Refused,
	);

	// `File(0)` against a regular file at size 0 is the regular-file landed
	// shape; the assertion is the regression net for the mask.
	assert_eq!(
		entry_landed(SentKind::File(0), "/tmp/x", Some(&stat(0, FILE_MODE))),
		LinkCheck::Confirmed,
	);

	// `File(4096)` against a directory is not a file either: a directory stats
	// at 4096 on most filesystems, the same size as the file.
	assert_eq!(
		entry_landed(SentKind::File(4096), "/tmp/x", Some(&stat(4096, DIR_MODE)),),
		LinkCheck::Refused,
	);

	// `Link` against a symlink where the runtime sent no `linkTarget` is
	// the pre-#1808 type check, and the destination satisfies it.
	assert_eq!(
		entry_landed(
			SentKind::Link("anywhere".to_string()),
			"/tmp/link",
			Some(&stat(7, LINK_MODE)),
		),
		LinkCheck::Fallback,
		"link confirmed by type because the runtime did not send linkTarget",
	);

	// `Link` against a regular file is not a link: a target the link points
	// at cannot satisfy the link confirmation, even at the right mode bits.
	assert_eq!(
		entry_landed(
			SentKind::Link("anywhere".to_string()),
			"/tmp/link",
			Some(&stat(7, FILE_MODE)),
		),
		LinkCheck::Refused,
	);
}

#[test]
fn a_file_landed_when_it_is_a_file_of_the_size_sent() {
	let sent = SentKind::File(4096);
	assert_eq!(
		entry_landed(sent.clone(), "/tmp/x", Some(&stat(4096, FILE_MODE))),
		LinkCheck::Confirmed,
	);
	assert_eq!(
		entry_landed(sent.clone(), "/tmp/x", Some(&stat(4095, FILE_MODE))),
		LinkCheck::Refused,
	);
	assert_eq!(
		entry_landed(sent.clone(), "/tmp/x", None),
		LinkCheck::Absent
	);
	// A directory stats at 4096 too. Same number, not the file.
	assert_eq!(
		entry_landed(sent.clone(), "/tmp/x", Some(&stat(4096, DIR_MODE))),
		LinkCheck::Refused,
	);
	// An empty file is a size like any other.
	assert_eq!(
		entry_landed(SentKind::File(0), "/tmp/x", Some(&stat(0, FILE_MODE))),
		LinkCheck::Confirmed,
	);
}

#[test]
fn a_directory_landed_when_a_directory_is_there() {
	assert_eq!(
		entry_landed(SentKind::Dir, "/tmp/x", Some(&stat(4096, DIR_MODE))),
		LinkCheck::Confirmed,
	);
	assert_eq!(
		entry_landed(SentKind::Dir, "/tmp/x", Some(&stat(0, DIR_MODE))),
		LinkCheck::Confirmed,
	);
	assert_eq!(
		entry_landed(SentKind::Dir, "/tmp/x", Some(&stat(4096, FILE_MODE))),
		LinkCheck::Refused,
	);
	assert_eq!(
		entry_landed(SentKind::Dir, "/tmp/x", None),
		LinkCheck::Absent
	);
}

/// The user-facing error for a refused entry names the entry's path. The
/// previous bool return forced the caller to invent a message that named
/// neither the entry nor the stat; this is the assertion the brief calls for.
#[test]
fn the_refusal_names_the_entry_path() {
	let failure = LandedFailure::Mismatch {
		path: "payload/link".to_string(),
		expected: SentKind::Link("anywhere".to_string()),
		stat: Some(stat(0, LINK_MODE)),
	};
	let msg = format_landed_failure(&failure, "/tmp");
	assert!(
		msg.contains("payload/link"),
		"the message must name the entry that failed: {msg}"
	);
	assert!(
		msg.contains("/tmp"),
		"the message must name the directory the archive was extracted at: {msg}"
	);
}

/// The user-facing error carries the literal PathStat the runtime answered,
/// not just a verdict. The intent is that the next attempt's log shows the
/// stat that was read for the failing entry, so a diagnosis can compare
/// expected and answered without re-running the upload.
#[test]
fn the_refusal_carries_the_stat_that_was_read() {
	let failure = LandedFailure::Mismatch {
		path: "payload/link".to_string(),
		expected: SentKind::Link("anywhere".to_string()),
		stat: Some(stat(0, LINK_MODE)),
	};
	let msg = format_landed_failure(&failure, "/tmp");
	// The Debug rendering of `PathStat` is what carries the stat; the field
	// names (size, mode) are part of the struct and end up in the message.
	assert!(
		msg.contains("size"),
		"the message must include the literal PathStat, whose Debug includes \
		 the field name `size`: {msg}"
	);
	assert!(
		msg.contains("mode"),
		"the message must include the literal PathStat, whose Debug includes \
		 the field name `mode`: {msg}"
	);
	// The expected kind is part of the message too: a future diagnosis needs
	// to know what the upload was supposed to land.
	assert!(
		msg.contains("Link"),
		"the message must name the entry kind that was expected: {msg}"
	);
}

/// A 404-without-stat shape (only reachable for symlinks on runtimes that
/// drop the header) reports the missing stat explicitly. `None` on its own
/// would be ambiguous to a reader who has not seen the dispatch.
#[test]
fn the_refusal_for_a_404_without_stat_explains_the_missing_stat() {
	let failure = LandedFailure::Mismatch {
		path: "payload/link".to_string(),
		expected: SentKind::Link("anywhere".to_string()),
		stat: None,
	};
	let msg = format_landed_failure(&failure, "/tmp");
	assert!(
		msg.contains("payload/link"),
		"the path is named even when the stat is missing: {msg}"
	);
	assert!(
		!msg.contains("None"),
		"`None` alone is ambiguous; the message must say no stat was returned: {msg}"
	);
	assert!(
		msg.contains("no stat"),
		"the message must explain the missing stat: {msg}"
	);
}

/// A stat request that errored out (a 5xx, a transport drop) reports the
/// runtime's message, so the next diagnosis can see why the verification
/// could not be done.
#[test]
fn the_refusal_for_a_stat_error_carries_the_runtime_message() {
	let failure = LandedFailure::StatError {
		path: "payload/link".to_string(),
		error: "podman transport: connection reset".to_string(),
	};
	let msg = format_landed_failure(&failure, "/tmp");
	assert!(msg.contains("payload/link"), "path is named: {msg}");
	assert!(
		msg.contains("connection reset"),
		"the runtime's message is carried verbatim: {msg}"
	);
}

/// An unreadable archive (the archive sent could not be parsed) reports the
/// reason only; no entry is named because no entry was reached.
#[test]
fn the_refusal_for_an_unreadable_archive_carries_the_reason() {
	let failure = LandedFailure::Unnamed("invalid gzip header".to_string());
	let msg = format_landed_failure(&failure, "/tmp");
	assert!(
		msg.contains("invalid gzip header"),
		"the reason is carried: {msg}"
	);
	assert!(
		!msg.contains("None"),
		"no stat is named because no stat was asked for: {msg}"
	);
}

// ---------------------------------------------------------------------------
// Link-target confirmation tests (#1808).
//
// The pre-#1808 check satisfied any symlink at the destination on the type
// bit alone; `entry_landed` now compares the value the tar header spelled
// against what Podman reports in `linkTarget`, after the lexical
// normalisation Podman applies to relative targets. The matrix below covers
// the six cases the brief names, each with its own assertion so a failure
// points at the row that regressed.
// ---------------------------------------------------------------------------

/// An absolute target that matches the runtime's `linkTarget` is confirmed
/// by the target. The `size` is the length of the link text, both halves
/// must agree.
#[test]
fn a_link_with_absolute_target_matching_is_confirmed() {
	let sent = SentKind::Link("/etc/hostname".to_string());
	let post = PathStat {
		size: 13,
		mode: LINK_MODE,
		link_target: "/etc/hostname".to_string(),
		..PathStat::default()
	};
	assert_eq!(
		entry_landed(sent, "/tmp/link-a", Some(&post)),
		LinkCheck::Confirmed,
		"matching target and matching size confirm the link by target",
	);
}

/// An absolute target that points elsewhere refuses the link. The
/// destination's `linkTarget` is the truth the runtime answered, not the
/// target the upload carried.
#[test]
fn a_link_with_absolute_target_pointing_elsewhere_is_refused() {
	let sent = SentKind::Link("/etc/hostname".to_string());
	let post = PathStat {
		size: 13,
		mode: LINK_MODE,
		link_target: "/etc/hosts".to_string(),
		..PathStat::default()
	};
	assert_eq!(
		entry_landed(sent, "/tmp/link-a", Some(&post)),
		LinkCheck::Refused,
		"the link points somewhere else; the upload did not land",
	);
}

/// A relative target sent as `../etc/hosts` from an entry at `/tmp/d/rel`
/// against `linkTarget: "/tmp/etc/hosts"` confirms. This is the
/// normalisation case: Podman reports the joined-and-resolved absolute
/// path; the sent side has to be normalised the same way before the
/// comparison, otherwise every relative link in a real tree would fail.
#[test]
fn a_link_with_relative_target_is_confirmed_after_normalisation() {
	let sent = SentKind::Link("../etc/hosts".to_string());
	let post = PathStat {
		size: 12,
		mode: LINK_MODE,
		link_target: "/tmp/etc/hosts".to_string(),
		..PathStat::default()
	};
	assert_eq!(
		entry_landed(sent, "/tmp/d/rel", Some(&post)),
		LinkCheck::Confirmed,
		"the relative target normalises to the same path Podman reports",
	);
}

/// A dangling relative target sent as `nothing-here` from an entry at
/// `/tmp/d/dangling` against `linkTarget: "/tmp/d/nothing-here"` confirms.
/// Podman resolves `..` lexically (and here there is none), but does not
/// stat the resolved path; the link is still confirmed because the link text
/// matches what the runtime put in the header.
#[test]
fn a_dangling_link_with_relative_target_is_confirmed_after_normalisation() {
	let sent = SentKind::Link("nothing-here".to_string());
	let post = PathStat {
		size: 12,
		mode: LINK_MODE,
		link_target: "/tmp/d/nothing-here".to_string(),
		..PathStat::default()
	};
	assert_eq!(
		entry_landed(sent, "/tmp/d/dangling", Some(&post)),
		LinkCheck::Confirmed,
		"the dangling link's text matches what Podman put in linkTarget",
	);
}

/// The normalised target matches but the byte length does not. The size
/// check is independent evidence: a runtime that misreports one without
/// the other still trips the comparison. A link that happens to point at
/// the right path after lex-resolution but carries a different byte length
/// (a different encoding of the same target, say) is refused.
#[test]
fn a_link_with_matching_target_but_wrong_size_is_refused() {
	let sent = SentKind::Link("../etc/hosts".to_string());
	let post = PathStat {
		// The target text is 12 bytes, but the runtime says 14. Either the
		// link text on disk is different from what the tar carried, or the
		// runtime is reporting the length of the resolved path. Both are
		// "the link that landed is not the link that was sent".
		size: 14,
		mode: LINK_MODE,
		link_target: "/tmp/etc/hosts".to_string(),
		..PathStat::default()
	};
	assert_eq!(
		entry_landed(sent, "/tmp/d/rel", Some(&post)),
		LinkCheck::Refused,
		"the size disagrees with the sent target's byte length",
	);
}

/// An empty `linkTarget` (an older runtime, or a Podman that stops
/// sending the field) is not a mismatch; it is the runtime's silence on
/// the question the confirmation needs answered. Fall back to the type
/// check that existed before #1808 and say so in the variant, so the log
/// can distinguish "confirmed by target" from "confirmed by type because
/// the runtime sent no target".
#[test]
fn a_link_with_empty_link_target_falls_back_to_type() {
	let sent = SentKind::Link("anywhere".to_string());
	let post = PathStat {
		size: 8,
		mode: LINK_MODE,
		link_target: String::new(),
		..PathStat::default()
	};
	assert_eq!(
		entry_landed(sent, "/tmp/link", Some(&post)),
		LinkCheck::Fallback,
		"empty linkTarget is the runtime's silence, not a wrong answer",
	);
}

/// A symlink at the destination that carries a `linkTarget` pointing at
/// something the upload did not send is refused even when the link text's
/// byte length matches. The byte length and the normalised path are
/// checked independently: getting one right without the other does not
/// pass.
#[test]
fn a_link_with_size_match_but_wrong_target_is_refused() {
	let sent = SentKind::Link("../etc/hosts".to_string());
	let post = PathStat {
		// The byte length matches the sent text (12), yet linkTarget
		// disagrees. The destination is a link, but it points at
		// something else.
		size: 12,
		mode: LINK_MODE,
		link_target: "/tmp/etc/hostname".to_string(),
		..PathStat::default()
	};
	assert_eq!(
		entry_landed(sent, "/tmp/d/rel", Some(&post)),
		LinkCheck::Refused,
		"the link text's byte length matching does not excuse a wrong target",
	);
}

/// The path the stat was taken at is the absolute container path of the
/// entry, not a relative name. The normalisation uses the parent of that
/// absolute path; passing a relative path here would yield a parent with no
/// leading `/`, and the join would put the target on the wrong side.
#[test]
fn a_relative_target_against_an_absolute_parent_normalises_correctly() {
	// Entry lives at `/srv/cfg/link`; sent target is `../etc/hosts`. The
	// parent is `/srv/cfg`, joined is `/srv/cfg/../etc/hosts`, normalised
	// is `/srv/etc/hosts`.
	let sent = SentKind::Link("../etc/hosts".to_string());
	let post = PathStat {
		size: 12,
		mode: LINK_MODE,
		link_target: "/srv/etc/hosts".to_string(),
		..PathStat::default()
	};
	assert_eq!(
		entry_landed(sent, "/srv/cfg/link", Some(&post)),
		LinkCheck::Confirmed,
		"the absolute container path's parent is the right join base",
	);
}

/// A symlink at the destination where the sent target's text matches but
/// the mode bits do not (e.g. the destination is a regular file at the
/// right size) is refused. The mode check runs first; the target check
/// would otherwise confirm against a target that lives at the right name
/// but is the wrong kind of entry.
#[test]
fn a_link_against_a_non_link_at_the_destination_is_refused() {
	let sent = SentKind::Link("/etc/hostname".to_string());
	let post = PathStat {
		size: 13,
		mode: FILE_MODE,
		link_target: "/etc/hostname".to_string(),
		..PathStat::default()
	};
	assert_eq!(
		entry_landed(sent, "/tmp/x", Some(&post)),
		LinkCheck::Refused,
		"a regular file at the destination is not a link, even at the right size",
	);
}

/// A backslash in a link target is an ordinary character in a container
/// path, not a separator, and the normalisation must say so on every host.
///
/// This is a platform-divergence guard, not a taste one. The first shape of
/// `lex_normalize` walked `std::path::Path::components`, which splits on
/// `\` on Windows and not on Unix. A link named `a\b` would then normalise
/// to one component on the Linux lane and two on the Windows lane, and a
/// `cp` verification would confirm or refuse depending on which host ran
/// `podup` rather than on what the runtime answered. `rust / Test
/// (windows-latest)` is a required check, so the divergence would have been
/// caught only if a test exercised this byte — and none did.
#[test]
fn a_backslash_in_a_target_is_not_a_separator() {
	let sent = SentKind::Link("a\\b".to_string());
	let post = PathStat {
		size: 3,
		mode: 1 << 27,
		mtime: String::new(),
		link_target: "/tmp/d/a\\b".to_string(),
	};
	assert_eq!(
		entry_landed(sent, "/tmp/d/link", Some(&post)),
		LinkCheck::Confirmed,
		"`a\\b` is one path component named `a\\b`, joined under /tmp/d"
	);
}
