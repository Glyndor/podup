use super::*;

// --- format_bytes --------------------------------------------------------

#[test]
fn format_bytes_scales_units() {
	assert_eq!(format_bytes(512), "512B");
	assert_eq!(format_bytes(1024), "1.0KiB");
	assert_eq!(format_bytes(1536), "1.5KiB");
	assert_eq!(format_bytes(1024 * 1024), "1.0MiB");
	assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0GiB");
}

/// The edges the scaling test steps over. It goes from 512 straight to 1024, so
/// the last value that must stay in bytes, and the empty case, were never
/// pinned, and a threshold with no test on either side of it is where an
/// off-by-one lives.
#[test]
fn format_bytes_pins_both_sides_of_the_first_threshold() {
	assert_eq!(format_bytes(0), "0B");
	assert_eq!(format_bytes(1), "1B");
	assert_eq!(format_bytes(1023), "1023B");
	assert_eq!(format_bytes(1024), "1.0KiB");
}

/// Every unit in the table, including the two the counters can only reach in
/// principle. `u64::MAX` is just under 16 EiB, so this also pins that the
/// function is total for its input type rather than saturating.
#[test]
fn format_bytes_reaches_the_top_of_the_table() {
	assert_eq!(format_bytes(1u64 << 40), "1.0TiB");
	assert_eq!(format_bytes(1u64 << 50), "1.0PiB");
	assert_eq!(format_bytes(1u64 << 60), "1.0EiB");
	// Before the table grew, this printed 16777216.0TiB.
	assert_eq!(format_bytes(u64::MAX), "16.0EiB");
}

/// A value just under a boundary used to round up onto it and print the unit it
/// had already left: one byte short of a mebibyte read `1024.0KiB`. Shared with
/// every other surface now, so a NET I/O counter crossing a rung says so.
#[test]
fn format_bytes_promotes_a_value_that_rounds_onto_the_next_unit() {
	assert_eq!(format_bytes(1024 * 1024 - 1), "1.0MiB");
	// The threshold is where the *rendered* value rounds up, so at one decimal
	// it is 1023.95 KiB and not the boundary itself. Both sides of it, since a
	// threshold with a test on only one side is where an off-by-one lives.
	assert_eq!(format_bytes(1024 * 1024 - 40), "1.0MiB");
	assert_eq!(format_bytes(1024 * 1024 - 100), "1023.9KiB");
	assert_eq!(format_bytes(1024 * 1000), "1000.0KiB");
}
