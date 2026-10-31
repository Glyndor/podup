use super::*;
use indexmap::IndexMap;

// EnvVars::to_map

#[test]
fn env_vars_empty_to_map() {
	assert!(EnvVars::Empty.to_map().is_empty());
}

#[test]
fn env_vars_list_key_equals_value() {
	let e = EnvVars::List(vec!["FOO=bar".into(), "BAZ=qux".into()]);
	let m = e.to_map();
	assert_eq!(m.get("FOO"), Some(&Some("bar".to_string())));
	assert_eq!(m.get("BAZ"), Some(&Some("qux".to_string())));
}

#[test]
fn env_vars_list_key_only_has_none_value() {
	let e = EnvVars::List(vec!["HOST_VAR".into()]);
	let m = e.to_map();
	assert_eq!(m.get("HOST_VAR"), Some(&None));
}

#[test]
fn env_vars_map_string_value() {
	let mut im = IndexMap::new();
	im.insert(
		"PORT".to_string(),
		Some(serde_yaml::Value::Number(8080.into())),
	);
	let m = EnvVars::Map(im).to_map();
	assert_eq!(m.get("PORT"), Some(&Some("8080".to_string())));
}

#[test]
fn env_vars_map_null_value_is_none() {
	let mut im = IndexMap::new();
	im.insert("X".to_string(), Some(serde_yaml::Value::Null));
	let m = EnvVars::Map(im).to_map();
	assert_eq!(m.get("X"), Some(&None));
}

#[test]
fn env_vars_is_empty_variants() {
	assert!(EnvVars::Empty.is_empty());
	assert!(EnvVars::List(vec![]).is_empty());
	assert!(!EnvVars::List(vec!["X=1".into()]).is_empty());
}

// EnvFileEntry

#[test]
fn env_file_entry_path_required_true() {
	let e = EnvFileEntry::Path(".env".into());
	assert_eq!(e.path(), ".env");
	assert!(e.required());
}

#[test]
fn env_file_entry_config_not_required() {
	let e = EnvFileEntry::Config {
		path: ".env.optional".into(),
		required: Some(false),
		format: None,
	};
	assert!(!e.required());
}

#[test]
fn env_file_entry_config_missing_required_defaults_true() {
	let e = EnvFileEntry::Config {
		path: ".env".into(),
		required: None,
		format: None,
	};
	assert!(e.required());
}

// EnvFile::to_entries / is_empty

#[test]
fn env_file_empty_to_entries() {
	assert!(EnvFile::Empty.to_entries().is_empty());
}

#[test]
fn env_file_single_to_entries() {
	let e = EnvFile::Single(EnvFileEntry::Path(".env".into()));
	assert_eq!(e.to_entries().len(), 1);
}

#[test]
fn env_file_list_to_list_returns_paths() {
	let e = EnvFile::List(vec![
		EnvFileEntry::Path(".env".into()),
		EnvFileEntry::Path(".env.local".into()),
	]);
	assert_eq!(e.to_list(), vec![".env", ".env.local"]);
}

#[test]
fn env_file_is_empty_variants() {
	assert!(EnvFile::Empty.is_empty());
	assert!(EnvFile::List(vec![]).is_empty());
	assert!(!EnvFile::Single(EnvFileEntry::Path(".env".into())).is_empty());
}

/// A list or a mapping as the value of an environment entry used to read as "no
/// value", which means "inherit from the host", so a malformed entry quietly
/// took whatever the host had under that name.
#[test]
fn an_environment_value_that_is_not_a_scalar_is_refused() {
	for bad in ["[bad]", "{a: 1}"] {
		let yaml =
			format!("services:\n  web:\n    image: x\n    environment:\n      SAMPLE: {bad}\n");
		assert!(crate::parse_str(&yaml).is_err(), "{bad} was accepted");
	}
	let ok = "services:\n  web:\n    image: x\n    environment:\n      A: 1\n      B: true\n      C: text\n      D:\n";
	assert!(crate::parse_str(ok).is_ok());
}
