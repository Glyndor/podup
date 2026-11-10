use super::super::build::BuildConfig;

fn ssh_of(yaml: &str) -> Vec<String> {
	let b: BuildConfig = serde_yaml::from_str(yaml).unwrap();
	b.ssh().to_vec()
}

#[test]
fn list_form_keeps_ids_and_key_paths() {
	assert_eq!(
		ssh_of("context: .\nssh:\n  - default\n  - mykey=/path/key\n"),
		["default", "mykey=/path/key"]
	);
}

#[test]
fn mapping_form_becomes_id_and_id_equals_path() {
	assert_eq!(
		ssh_of("context: .\nssh:\n  default:\n  mykey: /path/key\n"),
		["default", "mykey=/path/key"]
	);
}
