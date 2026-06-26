use podup::substitute::{build_vars, load_dotenv};
use std::io::Write;

#[test]
fn basic_key_value() {
	let dir = tempfile::tempdir().unwrap();
	let mut f = std::fs::File::create(dir.path().join(".env")).unwrap();
	writeln!(f, "# comment").unwrap();
	writeln!(f).unwrap();
	writeln!(f, "KEY=value").unwrap();
	writeln!(f, "EMPTY=").unwrap();
	// A bare key takes its value from the host env; unset there, it is omitted.
	writeln!(f, "PODUP_SUBST_DOTENV_NOVALUE").unwrap();
	std::env::remove_var("PODUP_SUBST_DOTENV_NOVALUE");

	let map = load_dotenv(dir.path());
	assert_eq!(map.get("KEY").map(|s| s.as_str()), Some("value"));
	assert_eq!(map.get("EMPTY").map(|s| s.as_str()), Some(""));
	assert!(!map.contains_key("PODUP_SUBST_DOTENV_NOVALUE"));
	assert!(!map.contains_key("# comment"));
}

#[test]
fn missing_file_returns_empty() {
	let dir = tempfile::tempdir().unwrap();
	assert!(load_dotenv(dir.path()).is_empty());
}

#[test]
fn process_env_takes_precedence() {
	let dir = tempfile::tempdir().unwrap();
	let mut f = std::fs::File::create(dir.path().join(".env")).unwrap();
	writeln!(f, "PATH=/should/not/override").unwrap();

	let map = load_dotenv(dir.path());
	assert!(!map.contains_key("PATH"));
}

#[test]
fn build_vars_includes_dotenv() {
	let dir = tempfile::tempdir().unwrap();
	let mut f = std::fs::File::create(dir.path().join(".env")).unwrap();
	writeln!(f, "PODUP_TEST_DOTENV_KEY=from_file").unwrap();

	let vars = build_vars(dir.path());
	assert_eq!(
		vars.get("PODUP_TEST_DOTENV_KEY").map(|s| s.as_str()),
		Some("from_file")
	);
}
