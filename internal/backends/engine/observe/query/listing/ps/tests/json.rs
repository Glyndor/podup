use super::*;

/// The JSON path carries the raw counts, and `null` when the size was not
/// requested, the same distinction the table draws, so a machine consumer can
/// tell "not asked" from "empty" too.
#[test]
fn the_json_row_distinguishes_an_absent_size_from_a_zero() {
	let absent = ps_json_row(&entry("", "running"));
	assert!(absent["Size"].is_null(), "{}", absent["Size"]);

	let c = ContainerListEntry {
		image_id: String::new(),
		size: Some(crate::libpod::types::container::ContainerSize {
			rw: 143_362,
			root_fs: 224_997_461,
		}),
		..entry("", "running")
	};
	let row = ps_json_row(&c);
	assert_eq!(row["Size"]["RwSize"], serde_json::json!(143_362));
	assert_eq!(row["Size"]["RootFsSize"], serde_json::json!(224_997_461));
}

/// The JSON path passes the wire values through rather than a rendering. A
/// machine consumer wants an instant it can compute with, and `docker compose ps
/// --format json` passes the RFC 3339 string through too.
#[test]
fn the_json_row_carries_the_raw_instants() {
	let c = ContainerListEntry {
		image_id: String::new(),
		started_at: 1_785_728_082,
		created: "2026-08-02T22:34:41.982670-05:00".into(),
		..entry("", "running")
	};
	let row = ps_json_row(&c);
	assert_eq!(row["StartedAt"], serde_json::json!(1_785_728_082_i64));
	assert_eq!(
		row["Created"],
		serde_json::json!("2026-08-02T22:34:41.982670-05:00")
	);
}

#[test]
fn ps_json_row_surfaces_state_exitcode_and_publishers() {
	let mut labels = HashMap::new();
	labels.insert("podup.project".to_string(), "demo".to_string());
	labels.insert("podup.service".to_string(), "web".to_string());
	let c = ContainerListEntry {
		image_id: String::new(),
		id: "deadbeef".into(),
		names: vec!["/demo-web-1".into()],
		image: "nginx:1.25".into(),
		status: "Exited (137) 2s ago".into(),
		state: "exited".into(),
		ports: vec![ContainerPort {
			host_ip: None,
			host_port: Some(8080),
			container_port: 80,
			protocol: Some("tcp".into()),
			range: None,
		}],
		exit_code: Some(137),
		labels,
		started_at: 1_785_728_082,
		created: "2026-08-02T22:34:41.982670-05:00".into(),
		size: None,
		is_infra: false,
	};
	let row = ps_json_row(&c);
	assert_eq!(row["Name"], "demo-web-1");
	assert_eq!(row["Service"], "web");
	assert_eq!(row["Project"], "demo");
	assert_eq!(row["State"], "exited");
	assert_eq!(row["ExitCode"], 137);
	assert_eq!(row["ID"], "deadbeef");
	assert_eq!(row["Publishers"][0]["PublishedPort"], 8080);
}
