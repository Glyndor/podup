use super::*;
use crate::parse_str_raw;

fn file(yaml: &str) -> ComposeFile {
	parse_str_raw(yaml).unwrap()
}

#[test]
fn clean_project_passes() {
	let f = file("services:\n  web:\n    image: nginx\nvolumes:\n  data:\nnetworks:\n  back:\n");
	validate_object_names(&f, "proj").unwrap();
}

#[test]
fn bad_explicit_volume_name_is_rejected() {
	let f = file(
		"services:\n  web:\n    image: nginx\nvolumes:\n  badvol:\n    name: \"x@bad name!\"\n",
	);
	let err = validate_object_names(&f, "proj").unwrap_err();
	let msg = err.to_string();
	assert!(msg.contains("invalid volume name"), "got: {msg}");
	assert!(msg.contains("x@bad name!"), "got: {msg}");
	assert!(msg.contains("badvol"), "got: {msg}");
}

#[test]
fn bad_default_volume_name_via_key_is_rejected() {
	// No explicit `name:`; the bad characters come from the key, so the
	// resolved name `proj_bad key` is rejected without a redundant origin.
	let f = file("services:\n  web:\n    image: nginx\nvolumes:\n  bad key:\n");
	let err = validate_object_names(&f, "proj").unwrap_err();
	assert!(
		err.to_string().contains("invalid volume name"),
		"got: {err}"
	);
}

#[test]
fn bad_explicit_network_name_is_rejected() {
	let f = file("services:\n  web:\n    image: nginx\nnetworks:\n  net:\n    name: \"bad@net\"\n");
	let err = validate_object_names(&f, "proj").unwrap_err();
	let msg = err.to_string();
	assert!(msg.contains("invalid network name"), "got: {msg}");
	assert!(msg.contains("bad@net"), "got: {msg}");
}

#[test]
fn bad_container_name_is_rejected() {
	let f = file("services:\n  web:\n    image: nginx\n    container_name: \"bad name!\"\n");
	let err = validate_object_names(&f, "proj").unwrap_err();
	assert!(
		err.to_string().contains("invalid container name"),
		"got: {err}"
	);
}

#[test]
fn external_resources_are_not_name_validated() {
	// An external resource is looked up by its name, not created, so the
	// strict create-time regex does not apply here.
	let f = file(
		"services:\n  web:\n    image: nginx\nvolumes:\n  ext:\n    external: true\n    name: \"weird:name\"\nnetworks:\n  en:\n    external: true\n    name: \"weird:net\"\n",
	);
	validate_object_names(&f, "proj").unwrap();
}

#[test]
fn origin_is_omitted_when_name_equals_key() {
	let err = ensure_valid_object_name("volume", "bad name", "bad name").unwrap_err();
	assert!(!err.to_string().contains("(from"), "got: {err}");
}

#[test]
fn a_control_character_in_a_key_or_a_name_does_not_reach_the_message() {
	for yaml in [
		"services:\n  \"a\\e[31mb\":\n    image: x\n",
		"services:\n  web:\n    image: x\n    container_name: \"a\\e[31mb\"\n",
		"services:\n  web:\n    image: x\nvolumes:\n  \"a\\e[31mb\":\n",
		"services:\n  web:\n    image: x\nnetworks:\n  \"a\\e[31mb\":\n",
		"services:\n  web:\n    image: x\nvolumes:\n  data:\n    name: \"a\\e[31mb\"\n",
	] {
		let text = validate_object_names(&file(yaml), "proj")
			.expect_err(yaml)
			.to_string();
		assert!(!text.contains('\u{1b}'), "{yaml}: {text:?}");
	}
}
