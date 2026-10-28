//! The failure replay: the lines a failed build puts back on screen.
//!
//! The replay exists so the reason a build failed is in front of the reader
//! (#1681), and it is the one path that prints the daemon's own build output
//! verbatim. That output is not ours to trust, and neither is the image tag
//! the lines are prefixed with.

use super::steps::replay_line;

/// The payload is a clear-screen, not an SGR: podup's own styling is always
/// `ESC [ … m`, so this sequence cannot be confused with it, and the assertion
/// holds whether or not colour is on.
#[test]
fn the_replayed_line_escapes_the_tag_and_the_build_output() {
	let out = replay_line("evil\u{1b}[2Jtag", "build\u{7}output");
	assert!(!out.contains("\u{1b}[2J"), "{out:?}");
	assert!(!out.contains('\u{7}'), "{out:?}");
	assert!(out.contains("\\u{1b}[2J"), "{out:?}");
	// Both parts are still there, and the separator is unchanged.
	assert!(out.contains("tag"), "{out:?}");
	assert!(out.contains("output"), "{out:?}");
	assert!(out.contains(" | "), "{out:?}");
}

/// An ordinary build line is untouched: the escaping must not cost a readable
/// reason its own characters.
#[test]
fn an_ordinary_build_line_is_replayed_verbatim() {
	assert_eq!(
		replay_line("localhost/img:1", "STEP 1/2: FROM alpine"),
		"localhost/img:1 | STEP 1/2: FROM alpine"
	);
}
