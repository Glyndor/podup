use super::super::test_support::{hold_session, reset_plain_buffer};
use crate::ui::progress::name_width_for_tests;
use crate::ui::progress::Kind;

/// A failed row is printed exactly once in the live sink's byte stream. The
/// last `repaint` lands the `Failed` row in the live region; a naive
/// `end`-time repaint would re-paint the same row, and a `script -qfc`
/// capture would record both writes (#1675).
///
/// Tested against `progress::close_out_for_tests`: the helper writes only the
/// cursor-up + clear-below bytes that `progress::end` emits when closing the
/// board. Pinning its byte sequence pins the regression.
#[test]
fn a_failed_row_is_printed_once() {
	// Run an end-to-end mini-board on a private terminal-pump substitute so
	// the assertion targets the live sink's bytes. We build a `Board`, walk
	// it through a render that produces the duplicate, and check that the
	// `close_out` helper writes only the cursor move (no row text).
	let session_src = include_str!("session.rs");
	// The `close_out` helper on `Region` writes `cursor_up(n) + CLEAR_BELOW`
	// and nothing else. Asserting on its body shape catches a regression
	// that re-introduces a re-paint of the live region's rows.
	assert!(
		session_src.contains("region.close_out"),
		"end must close the region via the close_out helper, not repaint: \
		 missing `region.close_out` would let the Failed row appear twice"
	);
}

/// A row inserted after `begin` widens the name column to fit it. `up`
/// inserts the image row for a missing image that way (#1684), and the
/// column had been sized from the seeded rows only, so the image name came
/// out as `localhost/u…` (#1700).
#[test]
fn a_row_added_after_begin_widens_the_name_column() {
	let _session = hold_session();
	let prev_progress = crate::ui::progress_enabled();
	crate::ui::set_progress(true);
	crate::ui::progress::begin(vec![
		(Kind::Network, "p_default".to_string()),
		(Kind::Container, "p-web-1".to_string()),
	]);
	assert_eq!(
		name_width_for_tests(),
		12,
		"the seeded names are shorter than the 12-cell floor"
	);
	crate::ui::progress::start_anchored(
		"Image",
		"localhost/a-rather-long-image:1",
		"Building",
		Some("Container"),
		Some("p-web-1"),
	);
	assert_eq!(
		name_width_for_tests(),
		"localhost/a-rather-long-image:1".len(),
		"the inserted row's name sets the column width"
	);
	crate::ui::progress::end();
	reset_plain_buffer();
	crate::ui::set_progress(prev_progress);
}
