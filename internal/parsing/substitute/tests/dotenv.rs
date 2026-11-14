use super::*;

// load_dotenv

#[test]
fn load_dotenv_strips_double_quoted_value() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join(".env"), "FOO=\"bar\"\n").unwrap();
	let map = load_dotenv(dir.path());
	assert_eq!(map.get("FOO").map(|s| s.as_str()), Some("bar"));
}

#[test]
fn load_dotenv_strips_single_quoted_value() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join(".env"), "FOO='bar'\n").unwrap();
	let map = load_dotenv(dir.path());
	assert_eq!(map.get("FOO").map(|s| s.as_str()), Some("bar"));
}

#[test]
fn load_dotenv_parses_key_value() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join(".env"), "FOO=bar\nBAZ=qux\n").unwrap();
	let map = load_dotenv(dir.path());
	assert_eq!(map.get("FOO").map(|s| s.as_str()), Some("bar"));
	assert_eq!(map.get("BAZ").map(|s| s.as_str()), Some("qux"));
}

#[test]
fn load_dotenv_skips_comments_and_blank_lines() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join(".env"), "# comment\n\nFOO=bar\n").unwrap();
	let map = load_dotenv(dir.path());
	assert_eq!(map.len(), 1);
	assert_eq!(map["FOO"], "bar");
}

#[test]
fn load_dotenv_bare_key_is_not_set_to_empty() {
	// A bare key (no `=`) no longer becomes an empty string. In `.env` it
	// resolves from the host, but `load_dotenv` already drops host-present
	// keys (process env wins), so either way a bare key never lands in the map
	// as `""`; it's host-provided for interpolation or absent.
	std::env::set_var("PODUP_DOTENV_ENV_PRESENT", "h");
	std::env::remove_var("PODUP_DOTENV_ENV_ABSENT");
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(
		dir.path().join(".env"),
		"PODUP_DOTENV_ENV_PRESENT\nPODUP_DOTENV_ENV_ABSENT\n",
	)
	.unwrap();
	let map = load_dotenv(dir.path());
	// host-present → dropped (process env wins); host-absent → omitted. Never "".
	assert!(!map.contains_key("PODUP_DOTENV_ENV_PRESENT"));
	assert!(!map.contains_key("PODUP_DOTENV_ENV_ABSENT"));
	std::env::remove_var("PODUP_DOTENV_ENV_PRESENT");
}

#[test]
fn load_dotenv_missing_file_returns_empty() {
	let dir = tempfile::tempdir().unwrap();
	let map = load_dotenv(dir.path());
	assert!(map.is_empty());
}

#[cfg(unix)]
#[test]
fn load_dotenv_follows_a_link_to_the_file() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("real.env"), "LINKED_ENV_KEY=3.19\n").unwrap();
	std::os::unix::fs::symlink("real.env", dir.path().join(".env")).unwrap();
	let map = load_dotenv(dir.path());
	assert_eq!(map.get("LINKED_ENV_KEY").map(|s| s.as_str()), Some("3.19"));
}

#[cfg(unix)]
#[test]
fn load_dotenv_ignores_a_link_that_leads_nowhere() {
	let dir = tempfile::tempdir().unwrap();
	std::os::unix::fs::symlink("missing.env", dir.path().join(".env")).unwrap();
	assert!(load_dotenv(dir.path()).is_empty());
}

/// `\$` in a double-quoted value is a literal dollar, as in Docker Compose: the
/// `$FOO` after it is not expanded, even with `FOO` set in the same file. The
/// backslash used to be dropped and the bare `$` expanded, so the value took
/// the contents of whatever variable followed.
#[test]
fn load_dotenv_keeps_an_escaped_dollar_literal() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(
		dir.path().join(".env"),
		"FOO=bar\nV=\"a\\$FOO\"\nW=\"a\\${FOO}\"\nX=\"a$FOO\"\n",
	)
	.unwrap();
	let map = load_dotenv(dir.path());
	assert_eq!(map["V"], "a$FOO");
	assert_eq!(map["W"], "a${FOO}");
	assert_eq!(map["X"], "abar", "an unescaped reference still expands");
}

/// A service `env_file:` goes through the same reader, so the escape holds there.
#[test]
fn a_service_env_file_keeps_an_escaped_dollar_literal() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("svc.env"), "FOO=bar\nV=\"a\\$FOO\"\n").unwrap();
	let entries = [crate::compose::types::EnvFileEntry::Path("svc.env".into())];
	let map = crate::env_file::load_env_file_entries(&entries, dir.path()).unwrap();
	assert_eq!(map["V"], "a$FOO");
}
