use super::*;

// --- how the output is painted -------------------------------------------

/// Colour has to vary with the value or it says nothing. Everything under 70%
/// used to be the same green, so a container at 0.02% looked exactly like one
/// at 69%.
#[test]
fn the_load_bands_separate_idle_from_busy() {
	let idle = load_style(0.02);
	let moderate = load_style(40.0);
	let warm = load_style(60.0);
	let hot = load_style(95.0);
	assert_ne!(idle, moderate, "0.02% must not look like 40%");
	assert_ne!(moderate, warm, "40% must not look like 60%");
	assert_ne!(warm, hot, "60% must not look like 95%");
}

/// `--format json` never repaints, whatever the terminal is. Asserted on the
/// pure predicate because no piped test can reach it: with stdout not a
/// terminal the other half of the condition is already false, so a version that
/// let JSON repaint on a tty passed the whole suite.
#[test]
fn the_machine_path_never_repaints() {
	assert!(wants_region(false, true), "a table on a terminal repaints");
	assert!(!wants_region(true, true), "json on a terminal must not");
	assert!(!wants_region(false, false), "nor a table off a terminal");
	assert!(!wants_region(true, false));
}
