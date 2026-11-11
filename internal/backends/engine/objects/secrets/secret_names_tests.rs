//! Which names a compose file may give a secret or a config.
//!
//! The Compose Specification keys them `[a-zA-Z0-9._-]+`, and the error says
//! "ASCII alphanumeric/dash/underscore/dot". The check reused the project-name
//! rule (lowercase, digits, `-` and `_` only), so `API_KEY` and `my.secret`,
//! which Docker Compose accepts, were refused with a message that listed them as
//! allowed.

use super::*;

fn source_name_of(kind: &str, key: &str) -> Result<String> {
	let yaml = format!(
		"services:\n  web:\n    image: x\n    {kind}s: ['{key}']\n{kind}s:\n  '{key}':\n    content: data\n"
	);
	let file = crate::compose::parse_str_raw(&yaml).unwrap();
	let plans = collect_native_plans("proj", &file.services["web"], &file, Path::new("/base"))?;
	Ok(plans[0].source.clone())
}

#[test]
fn uppercase_dot_dash_and_underscore_names_are_accepted() {
	for key in ["API_KEY", "my.secret", "db-pass", "Db", "a", "v1.2_x-Y"] {
		let source = source_name_of("secret", key)
			.unwrap_or_else(|e| panic!("{key:?} must be accepted: {e}"));
		assert_eq!(source, format!("proj_secret_{key}"));
	}
}

#[test]
fn the_same_names_are_accepted_for_a_config() {
	for key in ["App.Conf", "nginx_conf"] {
		let source = source_name_of("config", key)
			.unwrap_or_else(|e| panic!("{key:?} must be accepted: {e}"));
		assert_eq!(source, format!("proj_config_{key}"));
	}
}

#[test]
fn names_that_could_escape_or_hide_are_still_refused() {
	let too_long = "a".repeat(129);
	for key in [
		".hidden",
		"..",
		"../evil",
		"a/b",
		"a b",
		"a:b",
		"ñandú",
		"a\\nb",
		too_long.as_str(),
	] {
		assert!(
			source_name_of("secret", key).is_err(),
			"{key:?} must be refused"
		);
	}
}

#[test]
fn a_name_of_exactly_128_characters_is_accepted() {
	let key = "a".repeat(128);
	assert!(source_name_of("secret", &key).is_ok());
}
