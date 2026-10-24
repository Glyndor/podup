use podup::compose::parse_file_with_env_files_interp;
use std::fs;
use tempfile::{tempdir, TempDir};

const VARIABLE: &str = "PODUP_EXTENDS_GREETING_2118";

fn external_files(chain: bool) -> TempDir {
	let dir = tempdir().unwrap();
	fs::create_dir_all(dir.path().join("bases/deeper")).unwrap();
	fs::write(
		dir.path().join(if chain { "bases/deeper/common.yml" } else { "bases/common.yml" }),
		format!("services:\n  base:\n    image: alpine\n    environment:\n      GREETING: '${{{VARIABLE}:-hello}}'\n"),
	)
	.unwrap();
	let base = if chain {
		fs::write(
			dir.path().join("bases/middle.yml"),
			"services:\n  base:\n    extends: {file: deeper/common.yml, service: base}\n",
		)
		.unwrap();
		"bases/middle.yml"
	} else {
		"bases/common.yml"
	};
	fs::write(
		dir.path().join("compose.yml"),
		format!("services:\n  web:\n    extends: {{file: {base}, service: base}}\n"),
	)
	.unwrap();
	dir
}

#[test]
fn external_extends_preserves_no_interpolate_in_single_and_chained_files() {
	temp_env::with_var_unset(VARIABLE, || {
		for chain in [false, true] {
			let dir = external_files(chain);
			let file =
				parse_file_with_env_files_interp(&dir.path().join("compose.yml"), &[], false)
					.unwrap();
			assert_eq!(
				file.services["web"].environment.to_map()["GREETING"].as_deref(),
				Some(format!("${{{VARIABLE}:-hello}}").as_str()),
				"chain={chain}"
			);
		}
	});
}

#[test]
fn external_extends_receives_env_files_in_single_and_chained_files() {
	temp_env::with_var_unset(VARIABLE, || {
		for chain in [false, true] {
			let dir = external_files(chain);
			let env = dir.path().join("vars.env");
			fs::write(&env, format!("{VARIABLE}=hi\n")).unwrap();
			let file = parse_file_with_env_files_interp(
				&dir.path().join("compose.yml"),
				&["vars.env".to_string()],
				true,
			)
			.unwrap();
			assert_eq!(
				file.services["web"].environment.to_map()["GREETING"].as_deref(),
				Some("hi"),
				"chain={chain}"
			);
		}
	});
}

#[test]
fn included_extends_receives_the_including_files_env_options() {
	temp_env::with_var_unset(VARIABLE, || {
		let dir = external_files(true);
		fs::write(
			dir.path().join("local.env"),
			format!("{VARIABLE}=included\n"),
		)
		.unwrap();
		fs::write(
			dir.path().join("root.yml"),
			"include:\n  - path: compose.yml\n    env_file: local.env\n",
		)
		.unwrap();
		for interpolate in [true, false] {
			let file =
				parse_file_with_env_files_interp(&dir.path().join("root.yml"), &[], interpolate)
					.unwrap();
			let expected = if interpolate {
				"included".to_string()
			} else {
				format!("${{{VARIABLE}:-hello}}")
			};
			assert_eq!(
				file.services["web"].environment.to_map()["GREETING"].as_deref(),
				Some(expected.as_str())
			);
		}
	});
}
