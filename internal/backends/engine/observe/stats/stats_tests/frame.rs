use super::*;

// --- the rows of a frame -------------------------------------------------

#[test]
fn frame_rows_keeps_running_and_adds_stopped_zeros_sorted() {
	let report = StatsReport {
		stats: vec![
			ContainerStat {
				name: "proj-web-1".into(),
				cpu: 1.0,
				..Default::default()
			},
			// Not in `running` → must be dropped (stale daemon sample).
			ContainerStat {
				name: "other".into(),
				..Default::default()
			},
		],
	};
	let mut running = HashSet::new();
	running.insert("proj-web-1".to_string());
	let stopped = vec!["proj-db-1".to_string()];
	let rows = frame_rows(&report, &running, &stopped);
	// Sorted: db (stopped) before web (running); the unrelated sample is gone.
	assert_eq!(rows.len(), 2);
	assert_eq!(rows[0].name, "proj-db-1");
	assert_eq!(rows[0].cpu, 0.0);
	assert_eq!(rows[1].name, "proj-web-1");
	assert_eq!(rows[1].cpu, 1.0);
}

#[test]
fn frame_rows_without_all_has_no_stopped_rows() {
	let report = StatsReport::default();
	let running = HashSet::new();
	// `--all` off → caller passes an empty `stopped` slice → no rows.
	let rows = frame_rows(&report, &running, &[]);
	assert!(rows.is_empty());
}
