use crate::ui::progress::{
	note_lines_for_tests, push_note_live_for_tests, Kind, MAX_NOTES_PER_ROW_FOR_TESTS,
};

/// A row's stream tail keeps only the last four lines. Five lines pushed in
/// leaves the tail holding lines 2..=5, in arrival order. That is the
/// shape the live region paints dimmed under the row (#1681): four is the
/// window that keeps a step's `--> <layer>` line and the `COMMIT` line on
/// screen together at the end of a build without the region growing past
/// what a short terminal shows.
///
/// Reaching the live-push path in a cargo test requires going around the
/// terminal check that `note_for` uses to decide live vs plain: the test
/// runner captures stderr, so `live_terminal()` is false, and a `note_for`
/// call routes to the plain-sink stderr write instead of the tail. The
/// internal `push_note_live` helper is the only thing that calls the
/// append-and-trim path, so the test exercises it directly to pin the
/// trim window.
#[test]
fn notes_keep_the_last_four_lines_per_row() {
	let mut notes = std::collections::HashMap::new();
	let mut push = |line: &str| {
		push_note_live_for_tests(&mut notes, Kind::Image, "localhost/img:1", line);
	};
	push("first");
	push("second");
	push("third");
	push("fourth");
	push("fifth");

	let tail = notes
		.get(&(Kind::Image, "localhost/img:1".to_string()))
		.expect("the row's tail is populated");
	assert_eq!(
		tail.iter().cloned().collect::<Vec<_>>(),
		vec![
			"second".to_string(),
			"third".to_string(),
			"fourth".to_string(),
			"fifth".to_string(),
		],
		"after five pushes the tail holds the last four, in order"
	);
	assert_eq!(
		tail.len(),
		MAX_NOTES_PER_ROW_FOR_TESTS,
		"the cap is enforced on every push"
	);
}

/// #1733: a builder stream chunk carrying several lines in one write was one
/// tail entry. The board counted it as one row and the terminal drew it as
/// several, so `cursor_up(painted)` landed short and every later repaint
/// erased too few lines. One build scrolled the same block down the terminal
/// about eighty times.
#[test]
fn a_multi_line_chunk_becomes_one_entry_per_line() {
	assert_eq!(
		note_lines_for_tests("a\nb\nc"),
		vec!["a", "b", "c"],
		"a chunk with newlines must not stay one entry"
	);
}

#[test]
fn a_crlf_chunk_leaves_no_stray_carriage_return() {
	// `split('\n')` leaves the `\r` on each part and the `trim()` removes it.
	// `lines()` would swallow it too, but it would also make the split and
	// the trimming one decision instead of two.
	assert_eq!(
		note_lines_for_tests("a\r\nb\r\n"),
		vec!["a", "b"],
		"a CRLF chunk must produce clean entries"
	);
}

#[test]
fn empty_lines_inside_a_chunk_are_dropped() {
	// Matches what a single blank line already did before the split.
	assert_eq!(
		note_lines_for_tests("a\n\n   \nb"),
		vec!["a", "b"],
		"blank parts must be dropped, as a lone blank line already was"
	);
}

#[test]
fn the_note_cap_still_holds_when_the_input_arrives_as_one_chunk() {
	// A single arbitrarily tall entry defeated MAX_NOTES_PER_ROW: the cap
	// counts entries, so one entry of seven lines was one line as far as it
	// was concerned.
	let mut notes = std::collections::HashMap::new();
	for line in note_lines_for_tests("1\n2\n3\n4\n5\n6\n7") {
		push_note_live_for_tests(&mut notes, Kind::Image, "localhost/img:1", line);
	}
	let tail = notes
		.get(&(Kind::Image, "localhost/img:1".to_string()))
		.expect("the row's tail is populated");
	assert_eq!(
		tail.iter().cloned().collect::<Vec<_>>(),
		vec![
			"4".to_string(),
			"5".to_string(),
			"6".to_string(),
			"7".to_string()
		],
		"the cap must see one entry per line"
	);
}
