use super::*;

/// Planted-stat matrix for `entry_landed`. One case per row, each in
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
