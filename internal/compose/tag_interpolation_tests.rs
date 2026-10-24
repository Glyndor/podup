use super::merge::{deserialize_with_merge_interp, interpolated_value};
use std::collections::HashMap;

#[test]
fn override_scalar_is_interpolated_before_typing() {
	let file = deserialize_with_merge_interp(
		"services:\n  web:\n    image: alpine\n    user: !override '${UID:-runner}'\n",
		Some(&HashMap::new()),
	)
	.unwrap();
	assert_eq!(file.services["web"].user.as_deref(), Some("runner"));
}

#[test]
fn reset_payload_is_interpolated_before_stripping() {
	let value =
		interpolated_value("value: !reset '${UID:-2000}'\n", Some(&HashMap::new())).unwrap();
	let serde_yaml::Value::Tagged(tagged) = &value["value"] else {
		panic!("reset directive was stripped too early");
	};
	assert_eq!(tagged.tag, "!reset");
	assert_eq!(tagged.value, serde_yaml::Value::Number(2000.into()));
}

#[test]
fn override_mapping_visits_nested_list_and_mapping() {
	let file = deserialize_with_merge_interp(
		"services:\n  web:\n    image: alpine\n    environment: !override {GREETING: '${GREETING:-hello}'}\n    command: !override ['echo', '${GREETING:-hello}']\n",
		Some(&HashMap::new()),
	)
	.unwrap();
	assert_eq!(
		file.services["web"].environment.to_map()["GREETING"].as_deref(),
		Some("hello")
	);
	assert!(!serde_yaml::to_string(&file).unwrap().contains("${"));
}

#[test]
fn tagged_child_is_interpolated_under_same_file_extends() {
	let mut file = deserialize_with_merge_interp(
		"services:\n  base:\n    image: alpine\n  web:\n    extends: base\n    user: !override '${PODUP_TAG_USER:-runner}'\n",
		Some(&HashMap::new()),
	)
	.unwrap();
	super::extends::resolve_extends_same_file(&mut file).unwrap();
	assert_eq!(file.services["web"].user.as_deref(), Some("runner"));
}
