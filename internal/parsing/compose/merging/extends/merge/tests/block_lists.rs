/// Merge two files with `parse_files_with_env_files` and return the service.
fn merged(base: &str, over: &str) -> crate::compose::types::Service {
	let dir = tempfile::tempdir().unwrap();
	let a = dir.path().join("a.yaml");
	let b = dir.path().join("b.yaml");
	std::fs::write(&a, base).unwrap();
	std::fs::write(&b, over).unwrap();
	crate::parsing::compose::parse_files_with_env_files(&[a, b], &[])
		.unwrap()
		.services
		.shift_remove("s")
		.unwrap()
}

#[test]
fn build_lists_compose_appends_are_appended_without_repeats() {
	let svc = merged(
		"services:\n  s:\n    image: x\n    build:\n      context: .\n      cache_from: [x1, same]\n      tags: [t1]\n",
		"services:\n  s:\n    build:\n      cache_from: [same, x2]\n      tags: [t2]\n",
	);
	let build = serde_json::to_value(svc.build.unwrap()).unwrap();
	assert_eq!(build["cache_from"], serde_json::json!(["x1", "same", "x2"]));
	assert_eq!(build["tags"], serde_json::json!(["t1", "t2"]));
}

#[test]
fn device_reservations_from_both_files_are_kept() {
	let svc = merged(
		"services:\n  s:\n    image: x\n    deploy:\n      resources:\n        reservations:\n          devices: [{driver: nvidia, count: 1, capabilities: [gpu]}]\n",
		"services:\n  s:\n    deploy:\n      resources:\n        reservations:\n          devices: [{driver: amd, count: 2, capabilities: [gpu]}]\n",
	);
	let deploy = serde_json::to_value(svc.deploy.unwrap()).unwrap();
	let drivers: Vec<_> = deploy["resources"]["reservations"]["devices"]
		.as_array()
		.unwrap()
		.iter()
		.map(|d| d["driver"].as_str().unwrap().to_string())
		.collect();
	assert_eq!(drivers, ["nvidia", "amd"]);
}

#[test]
fn a_healthcheck_test_list_is_still_replaced() {
	let svc = merged(
		"services:\n  s:\n    image: x\n    healthcheck:\n      test: [CMD, a]\n      interval: 5s\n",
		"services:\n  s:\n    healthcheck:\n      test: [CMD, b]\n",
	);
	let hc = serde_json::to_value(svc.healthcheck.unwrap()).unwrap();
	assert_eq!(hc["test"], serde_json::json!(["CMD", "b"]));
	assert_eq!(hc["interval"], "5s");
}

#[test]
fn a_new_logging_driver_drops_the_old_drivers_options() {
	let svc = merged(
		"services:\n  s:\n    image: x\n    logging: {driver: json-file, options: {max-size: 1m}}\n",
		"services:\n  s:\n    logging: {driver: syslog}\n",
	);
	let logging = serde_json::to_value(svc.logging.unwrap()).unwrap();
	assert_eq!(logging["driver"], "syslog");
	assert!(
		logging.get("options").is_none_or(|o| o.is_null()),
		"{logging}"
	);
}

#[test]
fn the_same_logging_driver_keeps_merging_options() {
	let svc = merged(
		"services:\n  s:\n    image: x\n    logging: {driver: json-file, options: {max-size: 1m}}\n",
		"services:\n  s:\n    logging: {driver: json-file, options: {max-file: '3'}}\n",
	);
	let logging = serde_json::to_value(svc.logging.unwrap()).unwrap();
	assert_eq!(logging["options"]["max-size"], "1m");
	assert_eq!(logging["options"]["max-file"], "3");
}
