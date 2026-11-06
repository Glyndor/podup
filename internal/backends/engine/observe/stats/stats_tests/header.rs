use super::*;

// --- the header over its columns -----------------------------------------

/// The header and the rows are built from one set of widths, so every label
/// sits over its own column.
///
/// They were two hand-maintained layouts that nothing checked against each
/// other, and they had drifted: measured against a representative row, `MEM %`
/// sat one column past where its data ended and `PIDS` began exactly where its
/// data stopped, so the label was entirely off the column it named.
#[test]
fn the_header_lines_up_with_the_rows() {
	let row = format_row_with(&sample_stat(), false, false).expect("the row renders");
	let header = header();
	assert_eq!(
		header.chars().count(),
		row.chars().count(),
		"header and row must be the same width\nH: {header:?}\nR: {row:?}"
	);
	// Each label ends no later than its column does, which is what "over its own
	// column" means for a right-aligned numeric cell.
	for label in ["CPU %", "MEM %", "PIDS"] {
		let at = header.find(label).expect("label present");
		assert!(
			at + label.len() <= row.chars().count(),
			"{label} runs past the row\nH: {header:?}\nR: {row:?}"
		);
	}
}

/// A right-aligned label ends where its cell ends. `PIDS` used to start there
/// instead, which put the whole label past its data.
#[test]
fn the_pids_label_sits_over_the_pids_column() {
	let header = header();
	assert!(
		header.trim_end().ends_with("PIDS"),
		"PIDS is the trailing column: {header:?}"
	);
	let row = format_row_with(&sample_stat(), false, false).expect("the row renders");
	assert_eq!(
		header.rfind("PIDS").unwrap() + "PIDS".len(),
		row.chars().count(),
		"the label must end where the column does"
	);
}
