use super::*;

// --- the network counters ------------------------------------------------

/// The network counters come from the daemon, and each interface's counter is
/// a `u64`, so two of them can add up to more than a `u64` holds. The sum was
/// unchecked: it panicked in a debug build and wrapped in a release one, so
/// the total the table (and the JSON) reported was not the traffic that
/// happened. A sum that does not fit is reported instead.
#[test]
fn network_counters_that_do_not_fit_are_reported() {
	let mut network = HashMap::new();
	network.insert(
		"eth0".to_string(),
		NetStat {
			rx: u64::MAX,
			tx: 0,
		},
	);
	network.insert("eth1".to_string(), NetStat { rx: 1, tx: 0 });
	let s = ContainerStat {
		name: "proj-web-1".into(),
		network,
		..Default::default()
	};
	let err = format_row_with(&s, false, false).expect_err("the sum does not fit");
	assert!(
		matches!(err, crate::error::ComposeError::StatsOverflow(_)),
		"the table must report the overflow: {err:?}"
	);
	// The machine path reports it too: a JSON row with a wrapped counter is
	// worse than no row, because the consumer cannot tell it apart from the
	// real number.
	let err = stat_json_row(&s).expect_err("the sum does not fit");
	assert!(
		matches!(err, crate::error::ComposeError::StatsOverflow(_)),
		"the JSON path must report the overflow: {err:?}"
	);
}

/// A sum that does fit is still the sum of every interface, and the two
/// counters are added independently.
#[test]
fn network_counters_that_fit_are_summed() {
	let mut network = HashMap::new();
	network.insert("eth0".to_string(), NetStat { rx: 10, tx: 100 });
	network.insert("eth1".to_string(), NetStat { rx: 20, tx: 200 });
	let s = ContainerStat {
		name: "proj-web-1".into(),
		network,
		..Default::default()
	};
	let row = stat_json_row(&s).expect("the row renders");
	assert_eq!(row["NetInput"], 30);
	assert_eq!(row["NetOutput"], 300);
}
