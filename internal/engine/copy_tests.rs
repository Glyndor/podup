use super::verify::entry_landed;
use super::{
	cp_destination_kind, join_archive_path, parse_endpoint, uploaded_entry_kind, CpDestinationKind,
};
use crate::engine::copy::verify::{LinkCheck, SentKind};
use crate::libpod::client::PathStat;

#[test]
fn join_archive_path_does_not_double_the_separator() {
	// The #1097 re-verify stats `<dir>/<entry>`; a dir already ending in `/`
	// (notably root) must not produce `//entry`, which libpod reads as a
	// different path and 404s, turning a landed copy into a false failure.
	assert_eq!(join_archive_path("/tmp", "f.txt"), "/tmp/f.txt");
	assert_eq!(join_archive_path("/tmp/", "f.txt"), "/tmp/f.txt");
	assert_eq!(join_archive_path("/", "f.txt"), "/f.txt");
}

#[test]
fn entry_landed_asks_whether_the_entry_matches_what_was_uploaded() {
	let want = SentKind::File(42);
	let stat = |size: u64| PathStat {
		size,
		..PathStat::default()
	};
	// The entry is there and is the size that was sent -> landed.
	assert_eq!(
		entry_landed(want.clone(), "/tmp/f.txt", Some(&stat(42))),
		LinkCheck::Confirmed,
	);
	// A failed PUT leaves the old entry, which is a different size.
	assert_eq!(
		entry_landed(want.clone(), "/tmp/f.txt", Some(&stat(41))),
		LinkCheck::Refused,
	);
	assert_eq!(
		entry_landed(want.clone(), "/tmp/f.txt", Some(&stat(0))),
		LinkCheck::Refused,
	);
	// The entry vanished, or never appeared.
	assert_eq!(entry_landed(want, "/tmp/f.txt", None), LinkCheck::Absent,);
}

/// The case the previous signal could not express, and the reason it
/// changed: copying the **same** file twice.
///
/// The old check required the destination's mtime to move. The archive sets
/// that mtime from the source, so re-copying an unchanged file leaves it
/// identical by construction (no resolution would have helped) and the
/// second copy was reported as a failure. Matching against what was uploaded
/// answers correctly.
#[test]
fn copying_an_unchanged_file_twice_is_confirmed() {
	let want = SentKind::File(42);
	let already_there = PathStat {
		size: 42,
		..PathStat::default()
	};
	assert_eq!(
		entry_landed(want, "/tmp/f.txt", Some(&already_there)),
		LinkCheck::Confirmed,
	);
}

/// The shape that goes into the comparison is the source file's real length,
/// and a directory has none. The same function returns
/// `SentKind::Link(target)` for a host symlink source without
/// `-L/--follow-link`, so the destination is checked as a link rather than
/// against the link target's size.
///
/// A mutation replacing the regular-file length with a constant survived
/// every other test here, because they all build the expectation by hand;
/// this is the only one that goes through the filesystem.
#[test]
fn the_expected_kind_comes_from_the_source() {
	let dir = tempfile::tempdir().unwrap();
	let file = dir.path().join("payload.bin");
	std::fs::write(&file, vec![7u8; 1234]).unwrap();
	assert_eq!(
		uploaded_entry_kind(&file, false),
		Some(SentKind::File(1234))
	);

	std::fs::write(&file, b"").unwrap();
	assert_eq!(
		uploaded_entry_kind(&file, false),
		Some(SentKind::File(0)),
		"an empty file has a size"
	);

	// A directory upload has nothing comparable, so it stays unverifiable
	// and fail-closed rather than confirming on the directory's own size.
	assert_eq!(uploaded_entry_kind(dir.path(), false), None);
	assert_eq!(uploaded_entry_kind(&dir.path().join("absent"), false), None);
}

/// A host symlink at the source, copied without `-L/--follow-link`, expects a
/// symlink at the destination rather than being verified against the link
/// target's size (which is what the previous size-only comparison asked
/// about).
#[cfg(unix)]
#[test]
fn a_symlink_source_without_follow_expects_a_link() {
	let dir = tempfile::tempdir().unwrap();
	let link = dir.path().join("dangling");
	std::os::unix::fs::symlink("nowhere", &link).unwrap();
	assert_eq!(
		uploaded_entry_kind(&link, false),
		Some(SentKind::Link("nowhere".to_string())),
		"the host symlink target becomes the link target the tar carries",
	);

	// Following links makes the packer store the target's contents instead,
	// and the expectation becomes the target's shape.
	let real = dir.path().join("real");
	std::fs::write(&real, vec![1u8, 2, 3, 4]).unwrap();
	let link_to_real = dir.path().join("link_to_real");
	std::os::unix::fs::symlink(&real, &link_to_real).unwrap();
	assert_eq!(
		uploaded_entry_kind(&link_to_real, true),
		Some(SentKind::File(4))
	);
}

/// Two copies inside one second, which is what #1270 measured on Podman 6:
/// the mtime string is identical either side of the PUT because the runtime
/// reports whole seconds, while the size moved. Under the old signal this
/// was three failures in six back-to-back copies.
#[test]
fn two_copies_in_the_same_second_are_told_apart_by_size() {
	let same_second = "2026-08-03T18:36:05Z";
	let before = PathStat {
		size: 14,
		mtime: same_second.into(),
		..PathStat::default()
	};
	let after = PathStat {
		size: 15,
		mtime: same_second.into(),
		..PathStat::default()
	};
	assert_eq!(before.mtime, after.mtime, "the fixture must share an mtime");
	// What was uploaded is the 15-byte version.
	assert_eq!(
		entry_landed(SentKind::File(15), "/tmp/f.txt", Some(&after)),
		LinkCheck::Confirmed,
	);
	// And the pre-PUT entry would not have satisfied it.
	assert_eq!(
		entry_landed(SentKind::File(15), "/tmp/f.txt", Some(&before)),
		LinkCheck::Refused,
	);
}

#[test]
fn parse_service_colon_path() {
	assert_eq!(parse_endpoint("web:/app/data"), Some(("web", "/app/data")));
}

#[test]
fn parse_local_path_no_colon() {
	assert_eq!(parse_endpoint("/tmp/file.txt"), None);
}

#[test]
fn parse_dash_is_local() {
	assert_eq!(parse_endpoint("-"), None);
}

#[cfg(windows)]
#[test]
fn parse_windows_drive_letter_is_local() {
	assert_eq!(parse_endpoint("C:\\Users\\foo"), None);
}

#[cfg(not(windows))]
#[test]
fn single_char_service_parses_on_unix() {
	// On Unix a one-character service name is valid; only Windows treats a
	// single-char prefix as a drive letter.
	assert_eq!(parse_endpoint("c:/tmp/file"), Some(("c", "/tmp/file")));
	assert_eq!(parse_endpoint("w:data"), Some(("w", "data")));
}

#[test]
fn parse_empty_service_or_path() {
	assert_eq!(parse_endpoint(":path"), None);
	assert_eq!(parse_endpoint("svc:"), None);
}

#[cfg(windows)]
#[test]
fn parse_windows_drive_letter_forward_slash() {
	assert_eq!(parse_endpoint("C:/Users/foo"), None);
}

#[test]
fn parse_service_with_relative_path() {
	assert_eq!(
		parse_endpoint("web:data/file.txt"),
		Some(("web", "data/file.txt"))
	);
}

#[test]
fn parse_service_name_with_dots() {
	assert_eq!(
		parse_endpoint("my.service:/app/config"),
		Some(("my.service", "/app/config"))
	);
}

#[test]
fn check_endpoint_rejects_dash() {
	let err = super::check_endpoint("-").unwrap_err();
	assert!(format!("{err}").contains("stdin/stdout"), "got: {err}");
}

#[test]
fn check_endpoint_rejects_empty_container_path() {
	let err = super::check_endpoint("web:").unwrap_err();
	assert!(
		format!("{err}").contains("empty container path"),
		"got: {err}"
	);
}

#[test]
fn check_endpoint_allows_normal_forms() {
	// A plain local path, a proper SERVICE:PATH, and a relative host path are
	// all fine (validation only rejects `-` and `SERVICE:`).
	assert!(super::check_endpoint("/tmp/file").is_ok());
	assert!(super::check_endpoint("web:/app/data").is_ok());
	assert!(super::check_endpoint("./local").is_ok());
}

/// The builders set the field they are named for and nothing else. Nothing
/// in the tree calls them (the CLI goes through `CpOptions::new`), so a
/// mutation sweep on 2026-09-02 replaced each with `Default::default()` and
/// every suite stayed green; this is the only thing that reads them.
#[test]
fn the_cp_option_builders_set_their_field() {
	use super::CpOptions;
	let base = CpOptions::new(None, false, false);
	assert_eq!(
		CpOptions::new(Some(3), false, false),
		base.clone().with_index(Some(3))
	);
	assert_eq!(
		CpOptions::new(None, true, false),
		base.clone().with_follow_link(true)
	);
	assert_eq!(CpOptions::new(None, false, true), base.with_archive(true));
}

/// #1736 follow-up: the routing that picks between the streaming
/// extractor and the buffered `extract_archive` was using `dst.is_dir()`
/// directly, which follows a symlink and reports a directory. The
/// streaming branch would then extract into the link target rather than
/// the destination the user named (#1736's fix closed the same hole
/// inside `extract_archive`; this test pins the same fix at the
/// routing site).
#[test]
fn cp_destination_kind_treats_a_real_directory_as_a_directory() {
	let dir = tempfile::tempdir().expect("tempdir");
	let target = dir.path().join("real");
	std::fs::create_dir(&target).expect("mkdir");
	assert!(matches!(
		cp_destination_kind(&target),
		CpDestinationKind::Directory
	));
}
#[cfg(unix)]
#[test]
fn cp_destination_kind_treats_a_symlink_to_a_directory_as_a_symlink() {
	// Without this, `dst.is_dir()` followed the link and reported `true`;
	// the streaming branch then ran `extract_tar_guarded(link_target, ..)`
	// through the symlink. The bytes would land wherever the link points,
	// not on the path the user named. Mirrors the fix #1736 applied inside
	// `extract_archive`.
	let dir = tempfile::tempdir().expect("tempdir");
	let real = dir.path().join("real");
	std::fs::create_dir(&real).expect("mkdir");
	let link = dir.path().join("link");
	std::os::unix::fs::symlink(&real, &link).expect("symlink");
	assert!(
		matches!(cp_destination_kind(&link), CpDestinationKind::Symlink),
		"the cp routing must refuse a symlink destination before the extractor is reached",
	);
}
#[test]
fn cp_destination_kind_treats_a_missing_path_as_not_a_directory() {
	let dir = tempfile::tempdir().expect("tempdir");
	let missing = dir.path().join("absent");
	assert!(matches!(
		cp_destination_kind(&missing),
		CpDestinationKind::NotADirectory
	));
}
#[test]
fn cp_destination_kind_treats_a_regular_file_as_not_a_directory() {
	let dir = tempfile::tempdir().expect("tempdir");
	let file = dir.path().join("file");
	std::fs::write(&file, b"x").expect("write");
	assert!(matches!(
		cp_destination_kind(&file),
		CpDestinationKind::NotADirectory
	));
}

/// The contents cue is the trailing `/.` (or bare `.`) docker and podman use
/// to copy a directory's contents rather than the directory itself. The cue
/// is detected on the source as written because `Path::new(src).file_name()`
/// drops it; these tests pin the rule, one shape per assertion, so a
/// regression points at the exact case that moved.
#[test]
fn has_dot_contents_suffix_payload_no_slash_is_not_the_cue() {
	assert!(!super::has_dot_contents_suffix("payload"));
}

#[test]
fn has_dot_contents_suffix_payload_with_trailing_slash_is_not_the_cue() {
	assert!(!super::has_dot_contents_suffix("payload/"));
}

#[test]
fn has_dot_contents_suffix_payload_slash_dot_is_the_cue() {
	assert!(super::has_dot_contents_suffix("payload/."));
}

#[test]
fn has_dot_contents_suffix_payload_slash_dot_slash_is_the_cue() {
	assert!(super::has_dot_contents_suffix("payload/./"));
}

#[test]
fn has_dot_contents_suffix_payload_slash_dot_double_slash_is_the_cue() {
	assert!(super::has_dot_contents_suffix("payload/.//"));
}

#[test]
fn has_dot_contents_suffix_bare_dot_is_the_cue() {
	assert!(super::has_dot_contents_suffix("."));
}

#[test]
fn has_dot_contents_suffix_bare_dot_slash_is_the_cue() {
	assert!(super::has_dot_contents_suffix("./"));
}

#[test]
fn has_dot_contents_suffix_double_dot_is_not_the_cue() {
	assert!(!super::has_dot_contents_suffix(".."));
}

#[test]
fn has_dot_contents_suffix_a_slash_double_dot_is_not_the_cue() {
	assert!(!super::has_dot_contents_suffix("a/.."));
}

#[test]
fn has_dot_contents_suffix_empty_string_is_not_the_cue() {
	assert!(!super::has_dot_contents_suffix(""));
}

/// Windows people type the contents cue with a backslash, which
/// `std::path::is_separator` reports as a separator on that target only.
#[cfg(windows)]
#[test]
fn has_dot_contents_suffix_payload_backslash_dot_is_the_cue() {
	assert!(super::has_dot_contents_suffix("payload\\."));
}

#[cfg(windows)]
#[test]
fn has_dot_contents_suffix_payload_backslash_dot_backslash_is_the_cue() {
	assert!(super::has_dot_contents_suffix("payload\\.\\"));
}

#[cfg(windows)]
#[test]
fn has_dot_contents_suffix_payload_backslash_dot_double_backslash_is_the_cue() {
	assert!(super::has_dot_contents_suffix("payload\\.\\\\"));
}

#[cfg(windows)]
#[test]
fn has_dot_contents_suffix_bare_dot_backslash_is_the_cue() {
	assert!(super::has_dot_contents_suffix(".\\"));
}

#[cfg(windows)]
#[test]
fn has_dot_contents_suffix_payload_trailing_backslash_is_not_the_cue() {
	assert!(!super::has_dot_contents_suffix("payload\\"));
}

#[cfg(windows)]
#[test]
fn has_dot_contents_suffix_payload_backslash_double_dot_is_not_the_cue() {
	assert!(!super::has_dot_contents_suffix("payload\\.."));
}

/// On Unix a backslash is an ordinary filename character, so the cue must
/// not fire on `payload\.`: that name really is a directory called
/// `payload\.`, and copying its contents would silently copy the wrong
/// thing.
#[cfg(unix)]
#[test]
fn has_dot_contents_suffix_payload_backslash_dot_is_not_the_cue_backslash_is_ordinary_on_unix() {
	assert!(!super::has_dot_contents_suffix("payload\\."));
}
