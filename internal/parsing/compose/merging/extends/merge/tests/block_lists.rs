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

/// The other lists `docker compose config` 5.7.0 appends across files; each of
/// these used to keep only the later file's list.
#[test]
fn every_list_compose_appends_keeps_both_files() {
	let svc = merged(
		"services:\n  s:\n    image: x\n    \
		 build: {context: ., platforms: [linux/amd64], cache_to: [\"type=local,dest=/a\"]}\n    \
		 deploy: {placement: {constraints: [a==1], preferences: [{spread: a}]}}\n    \
		 develop: {watch: [{path: ./a, action: sync, target: /a}]}\n    \
		 gpus: [{driver: nvidia, count: 1}]\n",
		"services:\n  s:\n    \
		 build: {platforms: [linux/arm64], cache_to: [\"type=local,dest=/b\"]}\n    \
		 deploy: {placement: {constraints: [b==2], preferences: [{spread: b}]}}\n    \
		 develop: {watch: [{path: ./b, action: rebuild}]}\n    \
		 gpus: [{driver: nvidia, count: 2}]\n",
	);
	let value = serde_json::to_value(&svc).unwrap();
	let len = |v: &serde_json::Value| v.as_array().map(Vec::len);
	assert_eq!(len(&value["build"]["platforms"]), Some(2), "{value}");
	assert_eq!(len(&value["build"]["cache_to"]), Some(2), "{value}");
	assert_eq!(
		len(&value["deploy"]["placement"]["constraints"]),
		Some(2),
		"{value}"
	);
	assert_eq!(
		len(&value["deploy"]["placement"]["preferences"]),
		Some(2),
		"{value}"
	);
	assert_eq!(len(&value["develop"]["watch"]), Some(2), "{value}");
	assert_eq!(len(&value["gpus"]), Some(2), "{value}");
}

/// `blkio_config` merges key by key and its device lists append, so a later
/// file that adds a rate limit does not drop the earlier `weight`.
#[test]
fn blkio_config_merges_key_by_key() {
	let svc = merged(
		"services:\n  s:\n    image: x\n    \
		 blkio_config: {weight: 300, device_read_bps: [{path: /dev/sda, rate: 1mb}]}\n",
		"services:\n  s:\n    blkio_config: {device_read_bps: [{path: /dev/sdb, rate: 2mb}]}\n",
	);
	let blkio = serde_json::to_value(svc.blkio_config.unwrap()).unwrap();
	assert_eq!(blkio["weight"], 300, "{blkio}");
	assert_eq!(
		blkio["device_read_bps"].as_array().map(Vec::len),
		Some(2),
		"{blkio}"
	);
}

/// A count or `all` is one value: the later file's wins, as before.
#[test]
fn a_gpu_count_still_replaces() {
	let svc = merged(
		"services:\n  s:\n    image: x\n    gpus: [{driver: nvidia, count: 1}]\n",
		"services:\n  s:\n    gpus: all\n",
	);
	assert_eq!(serde_json::to_value(svc.gpus.unwrap()).unwrap(), "all");
}
