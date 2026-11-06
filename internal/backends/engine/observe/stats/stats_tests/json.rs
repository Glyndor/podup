use super::*;

// --- the JSON row --------------------------------------------------------

#[test]
fn stat_json_row_emits_numeric_fields() {
	let mut network = HashMap::new();
	network.insert("eth0".to_string(), NetStat { rx: 100, tx: 200 });
	let s = ContainerStat {
		name: "proj-db-1".into(),
		cpu: 5.5,
		mem_usage: 2048,
		mem_limit: 4096,
		mem_perc: 50.0,
		block_in: 1,
		block_out: 2,
		pids: 7,
		network,
	};
	let row = stat_json_row(&s).expect("the row renders");
	assert_eq!(row["Name"], "proj-db-1");
	assert_eq!(row["CPUPerc"], 5.5);
	assert_eq!(row["MemUsage"], 2048);
	assert_eq!(row["MemLimit"], 4096);
	assert_eq!(row["NetInput"], 100);
	assert_eq!(row["NetOutput"], 200);
	assert_eq!(row["PIDs"], 7);
}
