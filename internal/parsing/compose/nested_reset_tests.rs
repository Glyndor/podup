//! `!reset` on one entry or sub-key inside a service key.

use std::path::PathBuf;

use super::merging::nested_resets::nested_resets;

const BASE: &str = "services:\n  s:\n    image: busybox\n    depends_on: [db, cache]\n    labels: {x: '1', y: '2'}\n    networks: {n1: {}, n2: {}}\n    healthcheck: {test: [CMD, 'true'], interval: 5s, timeout: 3s}\n    deploy: {resources: {limits: {cpus: '1', memory: 100M}}}\n    environment: {A: a, B: b}\n  db: {image: busybox}\n  cache: {image: busybox}\nnetworks: {n1: {}, n2: {}}\n";

fn merged(over: &str) -> crate::compose::types::ComposeFile {
	let dir = tempfile::tempdir().unwrap();
	let base: PathBuf = dir.path().join("base.yaml");
	let over_path: PathBuf = dir.path().join("over.yaml");
	std::fs::write(&base, BASE).unwrap();
	std::fs::write(&over_path, format!("services:\n  s:\n    {over}\n")).unwrap();
	super::parse_files_with_env_files(&[base, over_path], &[]).unwrap()
}

#[test]
fn reset_removes_one_depends_on_entry() {
	for form in ["{db: !reset {}}", "{db: !reset null}"] {
		let file = merged(&format!("depends_on: {form}"));
		let deps = serde_yaml::to_value(&file.services["s"].depends_on).unwrap();
		let text = serde_yaml::to_string(&deps).unwrap();
		assert!(
			text.contains("cache") && !text.contains("db"),
			"{form}: {text}"
		);
	}
}

#[test]
fn reset_removes_one_label_and_one_environment_variable() {
	let file = merged("labels: {x: !reset null}\n    environment: {A: !reset null}");
	let s = &file.services["s"];
	let labels = serde_yaml::to_string(&s.labels).unwrap();
	assert!(labels.contains('y') && !labels.contains("x:"), "{labels}");
	let env = serde_yaml::to_string(&s.environment).unwrap();
	assert!(env.contains("B") && !env.contains("A:"), "{env}");
}

#[test]
fn reset_removes_one_network_of_the_service() {
	let file = merged("networks: {n1: !reset null}");
	let nets = serde_yaml::to_string(&file.services["s"].networks).unwrap();
	assert!(nets.contains("n2") && !nets.contains("n1"), "{nets}");
}

#[test]
fn reset_removes_one_healthcheck_field_and_keeps_the_rest() {
	let file = merged("healthcheck: {interval: !reset null}");
	let hc = file.services["s"].healthcheck.as_ref().unwrap();
	assert!(hc.interval.is_none());
	assert!(hc.timeout.is_some() && hc.test.is_some());
}

#[test]
fn reset_removes_a_nested_deploy_block() {
	for form in ["resources: !reset {}", "resources: {limits: !reset null}"] {
		let file = merged(&format!("deploy: {{{form}}}"));
		let limits = file.services["s"]
			.deploy
			.as_ref()
			.and_then(|d| d.resources.as_ref())
			.and_then(|r| r.limits.as_ref());
		assert!(limits.is_none(), "{form}");
	}
}

#[test]
fn an_override_without_a_reset_still_merges() {
	let file = merged("labels: {z: '3'}");
	let labels = serde_yaml::to_string(&file.services["s"].labels).unwrap();
	assert!(labels.contains('x') && labels.contains('z'), "{labels}");
}

#[test]
fn the_paths_are_read_from_the_raw_document() {
	let raw: serde_yaml::Value = serde_yaml::from_str(
		"services:\n  s:\n    depends_on: {db: !reset {}}\n    ports: !reset []\n    deploy: {resources: {limits: !reset null}}\n    labels: !override {a: b}\n",
	)
	.unwrap();
	let mut paths = nested_resets(&raw).remove("s").unwrap();
	paths.sort();
	assert_eq!(
		paths,
		vec![
			vec!["depends_on".to_string(), "db".to_string()],
			vec!["deploy".into(), "resources".into(), "limits".into()],
		]
	);
}
