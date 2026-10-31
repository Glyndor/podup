//! The bold header every list command prints above its data.

use super::{write_bold_header, write_bold_header_with};

/// A header written to a plain sink carries no escape. The header goes to a
/// raw sink, so nothing strips the styling for it: `--ansi never`, `NO_COLOR`
/// and a redirected stdout all asked for plain output and got bold escapes.
#[test]
fn a_plain_header_carries_no_escape() {
	let mut buf: Vec<u8> = Vec::new();
	write_bold_header_with(&mut buf, "NAME", false).expect("the header writes");
	let text = String::from_utf8_lossy(&buf);
	assert!(!text.contains('\u{1b}'), "{text:?}");
	// The label itself is still there.
	assert!(text.contains("NAME"), "{text:?}");
}

/// A colour sink keeps the styling: the header is the one row that says what
/// the columns are, and it reads as a header because it is bold.
#[test]
fn a_colour_header_keeps_its_styling() {
	let mut buf: Vec<u8> = Vec::new();
	write_bold_header_with(&mut buf, "NAME", true).expect("the header writes");
	let text = String::from_utf8_lossy(&buf);
	assert!(text.contains('\u{1b}'), "{text:?}");
	assert!(text.contains("NAME"), "{text:?}");
}

/// The real entry point follows the process-wide colour choice rather than
/// deciding for itself, so `--ansi never` and a redirected stdout reach every
/// header the same way they reach every other styled line.
#[test]
fn the_header_follows_the_colour_choice() {
	let mut real: Vec<u8> = Vec::new();
	write_bold_header(&mut real, "NAME").expect("the header writes");
	let mut expected: Vec<u8> = Vec::new();
	write_bold_header_with(&mut expected, "NAME", crate::ui::stdout_colored())
		.expect("the header writes");
	assert_eq!(
		real, expected,
		"the header must be written with the resolved colour choice"
	);
}
