use podup::compose::{parse_file_with_env_files_interp, parse_str_raw};
use std::fs;
use tempfile::tempdir;

#[test]
fn ordinary_include_keeps_its_existing_env_directory() {
	temp_env::with_var_unset("PODUP_INCLUDE_DIRECTORY_2118", || {
		let dir = tempdir().unwrap();
		fs::create_dir(dir.path().join("parts")).unwrap();
		fs::write(
			dir.path().join("vars.env"),
			"PODUP_INCLUDE_DIRECTORY_2118=root\n",
		)
		.unwrap();
		fs::write(
			dir.path().join("parts/vars.env"),
			"PODUP_INCLUDE_DIRECTORY_2118=included\n",
		)
		.unwrap();
		fs::write(
			dir.path().join("compose.yml"),
			"include: [parts/included.yml]\n",
		)
		.unwrap();
		fs::write(dir.path().join("parts/included.yml"), "services:\n  web:\n    image: alpine\n    environment: {VALUE: '${PODUP_INCLUDE_DIRECTORY_2118}'}\n").unwrap();
		let file = parse_file_with_env_files_interp(
			&dir.path().join("compose.yml"),
			&["vars.env".to_string()],
			true,
		)
		.unwrap();
		assert_eq!(
			file.services["web"].environment.to_map()["VALUE"].as_deref(),
			Some("included")
		);
	});
}

#[test]
fn included_child_inherits_the_parents_winning_base() {
	let dir = tempdir().unwrap();
	fs::write(
		dir.path().join("compose.yml"),
		"include: [included.yml]\nservices:\n  base:\n    image: alpine:3.20\n",
	)
	.unwrap();
	fs::write(
		dir.path().join("included.yml"),
		"services:\n  base:\n    image: busybox:1.36\n  web:\n    extends: base\n",
	)
	.unwrap();
	let file =
		parse_file_with_env_files_interp(&dir.path().join("compose.yml"), &[], true).unwrap();
	assert_eq!(file.services["web"].image.as_deref(), Some("alpine:3.20"));
}

#[test]
fn discarded_included_service_is_not_resolved() {
	let dir = tempdir().unwrap();
	fs::write(
		dir.path().join("compose.yml"),
		"include: [included.yml]\nservices:\n  web:\n    image: alpine:3.20\n",
	)
	.unwrap();
	fs::write(
		dir.path().join("included.yml"),
		"services:\n  web:\n    extends: missing\n",
	)
	.unwrap();
	let file =
		parse_file_with_env_files_interp(&dir.path().join("compose.yml"), &[], true).unwrap();
	assert_eq!(file.services["web"].image.as_deref(), Some("alpine:3.20"));
}

#[test]
fn numeric_users_keep_the_original_type_boundary() {
	for user in ["1000", "1000.5", ".inf", ".nan"] {
		assert!(
			parse_str_raw(&format!(
				"services:\n  web:\n    image: alpine\n    user: {user}\n"
			))
			.is_err(),
			"accepted {user}"
		);
	}
	assert_eq!(
		parse_str_raw("services:\n  web:\n    user: '1000'\n")
			.unwrap()
			.services["web"]
			.user
			.as_deref(),
		Some("1000")
	);
}

#[test]
fn included_external_parse_error_names_the_failing_base() {
	let dir = tempdir().unwrap();
	fs::create_dir(dir.path().join("bases")).unwrap();
	fs::write(dir.path().join("compose.yml"), "include: [included.yml]\n").unwrap();
	fs::write(
		dir.path().join("included.yml"),
		"services:\n  web:\n    extends: {file: bases/bad.yml, service: base}\n",
	)
	.unwrap();
	fs::write(dir.path().join("bases/bad.yml"), "services: [\n").unwrap();
	let error = parse_file_with_env_files_interp(&dir.path().join("compose.yml"), &[], true)
		.unwrap_err()
		.to_string();
	assert!(error.contains("bases/bad.yml"), "{error}");
	assert!(!error.contains("included.yml"), "{error}");
}
