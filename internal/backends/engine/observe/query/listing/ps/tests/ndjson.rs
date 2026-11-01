use super::super::ndjson::render;
use super::*;

#[test]
fn ndjson_has_one_compose_object_per_line_sorted_by_name() {
	let mut z = entry("Up 1 minute (healthy)", "running");
	z.names = vec!["/z-web-1".into()];
	z.created = "2026-08-03T00:58:00Z".into();
	z.labels.insert("podup.service".into(), "web".into());
	z.labels.insert("podup.project".into(), "z".into());
	z.ports = vec![ContainerPort {
		host_ip: None,
		host_port: Some(8080),
		container_port: 80,
		protocol: Some("tcp".into()),
		range: None,
	}];
	let mut a = entry("Exited (7)", "exited");
	a.names = vec!["/a-db-1".into()];
	a.exit_code = Some(7);
	let containers = [z, a];
	let z = &containers[0];
	let output = render(&containers, NOW).unwrap();
	assert!(output.ends_with('\n'));
	let rows: Vec<serde_json::Value> = output
		.lines()
		.map(|line| serde_json::from_str(line).unwrap())
		.collect();
	assert_eq!(rows.len(), 2);
	assert_eq!(rows[0]["Name"], "a-db-1");
	assert_eq!(rows[1]["Name"], "z-web-1");
	let keys = [
		"Command",
		"CreatedAt",
		"ExitCode",
		"Health",
		"ID",
		"Image",
		"Labels",
		"Name",
		"Names",
		"Networks",
		"Ports",
		"Project",
		"Publishers",
		"RunningFor",
		"Service",
		"Size",
		"State",
		"Status",
	];
	for row in &rows {
		assert_eq!(row.as_object().unwrap().len(), keys.len());
		for key in keys {
			assert!(row.get(key).is_some(), "missing {key}");
		}
	}
	assert_eq!(rows[0]["ExitCode"], 7);
	assert_eq!(rows[0]["Command"], "");
	assert_eq!(rows[0]["Networks"], "");
	assert_eq!(rows[0]["Size"], "");
	assert!(rows[0]["Publishers"].is_null());
	assert_eq!(rows[1]["Service"], "web");
	assert_eq!(rows[1]["Project"], "z");
	assert_eq!(rows[1]["Health"], "healthy");
	assert_eq!(rows[1]["Ports"], format_ports(&z.ports));
	assert_eq!(rows[1]["CreatedAt"], z.created);
	assert!(render(&[], NOW).unwrap().is_empty());
}

#[test]
fn legacy_json_bytes_and_constructor_remain_unchanged() {
	let row = ps_json_row(&entry("", "running"));
	assert_eq!(
		serde_json::to_string(&row).unwrap(),
		r#"{"Created":"","ExitCode":0,"Health":"","ID":"abc123","Image":"alpine","Name":"web","Project":"","Publishers":[],"Service":"","Size":null,"StartedAt":0,"State":"running","Status":"running"}"#
	);
	let opts = PsOptions::new(true, false, true);
	assert!(opts.json);
	assert!(!opts.ndjson);
	assert!(opts.with_ndjson(true).ndjson);
	assert!(
		!PsOptions::default()
			.with_ndjson(true)
			.with_json(true)
			.ndjson
	);
}
