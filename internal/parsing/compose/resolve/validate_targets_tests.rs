use crate::compose::ComposeFile;
use crate::parse_str;

fn check(yaml: &str) -> crate::error::Result<()> {
	let mut file: ComposeFile = parse_str(yaml).unwrap();
	crate::compose::normalize_default_network(&mut file);
	super::validate(&file)
}

#[test]
fn an_empty_short_volume_target_names_the_service_and_the_mount() {
	let err = check("services:\n  s:\n    image: x\n    volumes:\n      - \"./host:\"\n")
		.expect_err("empty target");
	let text = err.to_string();
	assert!(text.contains("service 's'"), "{text}");
	assert!(text.contains("volumes[0]"), "{text}");
	assert!(text.contains("./host:"), "{text}");
}

#[test]
fn an_unset_variable_in_the_target_is_refused_before_anything_runs() {
	let yaml = "services:\n  s:\n    image: x\n    volumes:\n      - ./host:${PODUP_TARGET_NOT_SET_ANYWHERE}\n";
	let err = temp_env::with_var_unset("PODUP_TARGET_NOT_SET_ANYWHERE", || check(yaml))
		.expect_err("empty target");
	assert!(err.to_string().contains("empty target"), "{err}");
}

#[test]
fn well_formed_short_volumes_still_pass() {
	let yaml = "services:\n  s:\n    image: x\n    volumes:\n      - ./a:/a\n      - /b\n      - named:/c:ro\nvolumes:\n  named: {}\n";
	check(yaml).unwrap();
}
