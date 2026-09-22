//! Link-target confirmation (#1808).
//!
//! `entry_landed` used to satisfy any symlink at the destination on the
//! type bit alone. It now compares the target the tar header spelled
//! against what Podman reports in `linkTarget`, after the lexical
//! normalisation Podman applies to relative targets, and compares `size`
//! against the byte length of the link text. One case per test, so a
//! failure points at the row that regressed.

use super::*;

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
