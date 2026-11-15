//! `!override` below a service key and on a top-level entry takes the later
//! value whole, as `docker compose config` 5.7.0 does, instead of merging it
//! key by key into the earlier file's.

use std::path::PathBuf;

const BASE: &str = "services:\n  s:\n    image: busybox\n    networks: [n2]\n    \
volumes: [\"v2:/v\"]\n    deploy: {resources: {limits: {cpus: \"1\", memory: 100M}}}\n\
networks:\n  n2: {driver: bridge, driver_opts: {o1: \"1\"}}\n\
volumes:\n  v2: {driver_opts: {o1: \"1\"}}\n";

fn merged(over: &str) -> crate::compose::types::ComposeFile {
	let dir = tempfile::tempdir().unwrap();
	let base: PathBuf = dir.path().join("base.yaml");
	let over_path: PathBuf = dir.path().join("over.yaml");
	std::fs::write(&base, BASE).unwrap();
	std::fs::write(&over_path, over).unwrap();
	super::parse_files_with_env_files(&[base, over_path], &[]).unwrap()
}

#[test]
fn nested_override_drops_what_the_earlier_file_set_there() {
	let file =
		merged("services:\n  s:\n    deploy: {resources: !override {limits: {memory: 200M}}}\n");
	let limits =
		serde_yaml::to_value(&file.services["s"]).unwrap()["deploy"]["resources"]["limits"].clone();
	assert!(limits.get("cpus").is_none(), "cpus kept: {limits:?}");
	assert!(limits.get("memory").is_some(), "memory lost: {limits:?}");
}

#[test]
fn override_on_a_network_and_a_volume_replaces_the_entry() {
	let file =
		merged("networks:\n  n2: !override {driver: bridge}\nvolumes:\n  v2: !override {}\n");
	let net = serde_yaml::to_value(&file.networks["n2"]).unwrap();
	assert!(
		net.get("driver_opts").is_none(),
		"network kept driver_opts: {net:?}"
	);
	let vol = serde_yaml::to_value(&file.volumes["v2"]).unwrap();
	assert!(
		vol.get("driver_opts").is_none(),
		"volume kept driver_opts: {vol:?}"
	);
}

#[test]
fn an_untagged_entry_still_merges_key_by_key() {
	let file = merged("networks:\n  n2: {labels: {b: \"2\"}}\n");
	let net = serde_yaml::to_value(&file.networks["n2"]).unwrap();
	assert!(net.get("driver_opts").is_some(), "{net:?}");
	assert!(net.get("labels").is_some(), "{net:?}");
}
