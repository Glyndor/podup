use super::*;

// --- the row of the table ------------------------------------------------

#[test]
fn format_row_sums_network_and_shows_name() {
	let mut network = HashMap::new();
	network.insert("eth0".to_string(), NetStat { rx: 1024, tx: 2048 });
	let s = ContainerStat {
		name: "proj-web".into(),
		cpu: 12.5,
		mem_usage: 1024 * 1024,
		mem_limit: 1024 * 1024 * 1024,
		mem_perc: 0.1,
		block_in: 0,
		block_out: 0,
		pids: 3,
		network,
	};
	let row = format_row_with(&s, false, false).expect("the row renders");
	assert!(row.contains("proj-web"));
	assert!(row.contains("12.50%"));
	assert!(row.contains("1.0MiB"));
	assert!(row.contains('3'));
}

#[test]
fn truncate_name_shortens_long_names_with_ellipsis() {
	let short = "proj-web-1";
	assert_eq!(truncate_name(short, false), short);
	// Exactly the column width is kept verbatim.
	let exact = "a".repeat(NAME_WIDTH);
	assert_eq!(truncate_name(&exact, false), exact);
	// One over the width is truncated to width-1 chars plus an ellipsis.
	let long = "a".repeat(NAME_WIDTH + 10);
	let cut = truncate_name(&long, false);
	assert_eq!(cut.chars().count(), NAME_WIDTH);
	assert!(cut.ends_with('…'));
	// `--no-trunc` keeps the full name regardless of length.
	assert_eq!(truncate_name(&long, true), long);
}

#[test]
fn format_row_truncates_long_name_but_no_trunc_keeps_it() {
	let s = ContainerStat {
		name: "really-long-project-web-container-name-1".into(),
		..Default::default()
	};
	// Default: the long name is truncated (ellipsis present, full name gone).
	let row = format_row_with(&s, false, false).expect("the row renders");
	assert!(row.contains('…'));
	assert!(!row.contains(&s.name));
	// `--no-trunc`: the full name survives intact.
	let row_full = format_row_with(&s, true, false).expect("the row renders");
	assert!(row_full.contains(&s.name));
}

/// The container name arrives from the daemon, so it is not ours: a raw escape
/// in it repaints the reader's terminal and desynchronises the table's own
/// resets. `stats` painted the name as it came, so the escape reached stdout
/// unchanged. The name has to be escaped before it is measured, truncated or
/// painted, so the width the column reserves is the width actually printed.
#[test]
fn a_container_name_cannot_drive_the_terminal() {
	let s = ContainerStat {
		name: "proj-web\u{1b}[31m\u{7}1".into(),
		..Default::default()
	};
	let row = format_row_with(&s, false, false).expect("the row renders");
	assert!(!row.contains('\u{1b}'), "{row:?}");
	assert!(!row.contains('\u{7}'), "{row:?}");
	// The name is still there, as text.
	assert!(row.contains("proj-web"), "{row:?}");
}

/// Escaping happens before the truncation, so the column keeps its width: a
/// name whose escape sequences expanded past the column must not push the
/// numbers that follow it sideways.
#[test]
fn the_escaped_name_is_measured_before_the_row_is_padded() {
	let s = ContainerStat {
		name: "proj-web\u{1b}1".into(),
		..Default::default()
	};
	let row = format_row_with(&s, false, false).expect("the row renders");
	// The escaped name is 8 + 6 + 1 = 15 columns, well inside NAME_WIDTH, so
	// the row is padded to the same width as any other name.
	assert_eq!(
		row.chars().count(),
		format_row_with(&sample_stat(), false, false)
			.expect("the row renders")
			.chars()
			.count(),
		"a name with escapes must not change the row's width: {row:?}"
	);
}
