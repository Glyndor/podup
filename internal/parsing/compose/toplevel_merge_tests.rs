//! A volume, network, secret or config that two files both declare merges key
//! by key, as `docker compose config` does, instead of the last file replacing it.

fn merged(base: &str, over: &str) -> crate::compose::types::ComposeFile {
	let dir = tempfile::tempdir().unwrap();
	let a = dir.path().join("a.yaml");
	let b = dir.path().join("b.yaml");
	std::fs::write(&a, base).unwrap();
	std::fs::write(&b, over).unwrap();
	super::parse_files_with_env_files(&[a, b], &[]).unwrap()
}

#[test]
fn a_network_keeps_its_driver_and_adds_the_labels_of_the_second_file() {
	let file = merged(
		"services:\n  s: {image: x}\nnetworks:\n  n: {driver: bridge, labels: {a: '1'}}\n",
		"networks:\n  n: {labels: {b: '2'}}\n",
	);
	let net = serde_json::to_value(file.networks["n"].as_ref().unwrap()).unwrap();
	assert_eq!(net["driver"], "bridge");
	assert_eq!(net["labels"]["a"], "1");
	assert_eq!(net["labels"]["b"], "2");
}

#[test]
fn a_volume_keeps_its_driver_when_the_second_file_adds_options() {
	let file = merged(
		"services:\n  s: {image: x}\nvolumes:\n  v: {driver: local, labels: {a: '1'}}\n",
		"volumes:\n  v: {labels: {b: '2'}}\n",
	);
	let vol = serde_json::to_value(file.volumes["v"].as_ref().unwrap()).unwrap();
	assert_eq!(vol["driver"], "local");
	assert_eq!(vol["labels"]["a"], "1");
	assert_eq!(vol["labels"]["b"], "2");
}

#[test]
fn a_secret_source_set_by_the_second_file_wins() {
	let file = merged(
		"services:\n  s: {image: x}\nsecrets:\n  k: {file: ./one}\n",
		"secrets:\n  k: {file: ./two}\n",
	);
	let secret = serde_json::to_value(&file.secrets["k"]).unwrap();
	assert_eq!(secret["file"], "./two");
}

#[test]
fn resources_only_one_file_declares_are_kept_in_order() {
	let file = merged(
		"services:\n  s: {image: x}\nnetworks:\n  first: {}\n  second: {}\n",
		"networks:\n  second: {driver: bridge}\n  third: {}\n",
	);
	let names: Vec<_> = file.networks.keys().take(3).cloned().collect();
	assert_eq!(names, ["first", "second", "third"]);
}
