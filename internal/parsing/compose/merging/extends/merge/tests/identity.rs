//! Entries with an identity (a mount target, a port binding, an env file path,
//! a dependency) are one entry across files, as `docker compose config` 5.7.0
//! reads them: the later file's replaces the earlier one in place.

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

const SECRETS: &str = "secrets: {x: {file: ./x}, y: {file: ./y}}\nconfigs: {x: {file: ./x}}\n";

#[test]
fn a_secret_or_config_at_the_same_target_is_one_entry() {
	let svc = merged(
		&format!(
			"services:\n  s:\n    image: x\n    secrets: [x, {{source: y, target: t1}}]\n    configs: [x]\n{SECRETS}"
		),
		"services:\n  s:\n    secrets: [{source: x, mode: 0400}, {source: y, target: t2}]\n    \
		 configs: [{source: x, uid: \"1\"}]\n",
	);
	let secrets = serde_json::to_value(&svc.secrets).unwrap();
	assert_eq!(secrets.as_array().map(Vec::len), Some(3), "{secrets}");
	assert_eq!(
		secrets[0]["mode"], 0o400,
		"the later entry replaces in place: {secrets}"
	);
	let configs = serde_json::to_value(&svc.configs).unwrap();
	assert_eq!(configs.as_array().map(Vec::len), Some(1), "{configs}");
}

#[test]
fn a_short_port_and_its_long_form_are_one_entry() {
	let svc = merged(
		"services:\n  s:\n    image: x\n    ports: [\"8080:80\", \"9090:90/udp\"]\n",
		"services:\n  s:\n    ports: [{target: 80, published: \"8080\", protocol: tcp}, \
		 {target: 90, published: \"9090\"}]\n",
	);
	// 8080/tcp is the same binding; 9090 tcp is not the udp one.
	assert_eq!(svc.ports.len(), 3, "{:?}", svc.ports);
	assert!(matches!(
		svc.ports[0],
		crate::compose::types::PortMapping::Long { .. }
	));
}

#[test]
fn a_dependency_merges_field_by_field() {
	let svc = merged(
		"services:\n  s:\n    image: x\n    depends_on: {d1: {condition: service_healthy, \
		 restart: true, required: false}}\n  d1: {image: x}\n  d3: {image: x}\n",
		"services:\n  s:\n    depends_on: [d1, d3]\n",
	);
	let deps = serde_json::to_value(&svc.depends_on).unwrap();
	assert_eq!(deps["d1"]["condition"], "service_started", "{deps}");
	assert_eq!(deps["d1"]["restart"], true, "restart must survive: {deps}");
	assert_eq!(
		deps["d1"]["required"], true,
		"a list entry is required: {deps}"
	);
	assert!(deps.get("d3").is_some(), "{deps}");
}

#[test]
fn the_later_env_file_entry_wins_for_the_same_path() {
	// e1.env does not exist: only the later `required: false` lets this load.
	let svc = merged(
		"services:\n  s:\n    image: x\n    env_file: [e1.env]\n",
		"services:\n  s:\n    env_file: [{path: e1.env, required: false}]\n",
	);
	let entries = svc.env_file.to_entries();
	assert_eq!(entries.len(), 1);
	let entry = serde_json::to_value(&entries[0]).unwrap();
	assert_eq!(entry["required"], false, "{entry}");
}
