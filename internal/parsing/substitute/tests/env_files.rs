use super::*;

// build_vars_with_env_files_strict

#[test]
fn env_file_replaces_dotenv() {
	let dir = tempfile::tempdir().unwrap();
	// Compose v2: an explicit `--env-file` replaces the default `.env`, so the
	// dotenv-only key is absent and the extra file's value wins for a shared key.
	std::fs::write(
		dir.path().join(".env"),
		"FROM_DOTENV=base\nPODUP_TEST_SHARED=dotenv\n",
	)
	.unwrap();
	std::fs::write(
		dir.path().join("extra.env"),
		"FROM_EXTRA=more\nPODUP_TEST_SHARED=extra\n",
	)
	.unwrap();

	let vars = build_vars_with_env_files_strict(dir.path(), &["extra.env".to_string()]).unwrap();
	assert_eq!(vars.get("FROM_DOTENV"), None);
	assert_eq!(vars.get("FROM_EXTRA").map(String::as_str), Some("more"));
	assert_eq!(
		vars.get("PODUP_TEST_SHARED").map(String::as_str),
		Some("extra")
	);
}

#[test]
fn later_env_file_wins() {
	let dir = tempfile::tempdir().unwrap();
	// Among several `--env-file`s the last one listed wins.
	std::fs::write(dir.path().join("a.env"), "FROM_A=a\nSHARED=a\n").unwrap();
	std::fs::write(dir.path().join("b.env"), "FROM_B=b\nSHARED=b\n").unwrap();

	let vars =
		build_vars_with_env_files_strict(dir.path(), &["a.env".to_string(), "b.env".to_string()])
			.unwrap();
	assert_eq!(vars.get("FROM_A").map(String::as_str), Some("a"));
	assert_eq!(vars.get("FROM_B").map(String::as_str), Some("b"));
	assert_eq!(vars.get("SHARED").map(String::as_str), Some("b"));
}

#[test]
fn process_env_wins_over_env_file() {
	let dir = tempfile::tempdir().unwrap();
	std::env::set_var("PODUP_ENVFILE_PROCESS_WINS", "from-process");
	std::fs::write(
		dir.path().join("x.env"),
		"PODUP_ENVFILE_PROCESS_WINS=from-file\n",
	)
	.unwrap();

	let vars = build_vars_with_env_files_strict(dir.path(), &["x.env".to_string()]).unwrap();
	assert_eq!(
		vars.get("PODUP_ENVFILE_PROCESS_WINS").map(String::as_str),
		Some("from-process")
	);
	std::env::remove_var("PODUP_ENVFILE_PROCESS_WINS");
}

#[test]
fn no_env_file_loads_dotenv() {
	let dir = tempfile::tempdir().unwrap();
	// With no `--env-file`, `.env` is loaded as before.
	std::fs::write(dir.path().join(".env"), "FROM_DOTENV=base\n").unwrap();
	let vars = build_vars_with_env_files_strict(dir.path(), &[]).unwrap();
	assert_eq!(vars.get("FROM_DOTENV").map(String::as_str), Some("base"));
}

#[test]
fn strict_build_vars_errors_on_missing_extra_file() {
	let dir = tempfile::tempdir().unwrap();
	// An explicitly-passed `--env-file` that does not exist is a hard error
	// (docker compose parity), not a silent fall-back to defaults.
	let err =
		build_vars_with_env_files_strict(dir.path(), &["absent.env".to_string()]).unwrap_err();
	assert!(
		matches!(err, crate::error::ComposeError::EnvFile(_)),
		"expected EnvFile error, got {err:?}"
	);
	assert!(err.to_string().contains("env file not found"));
}

#[test]
fn strict_build_vars_errors_on_unterminated_quote() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("bad.env"), "A=\"oops\nB=keep\n").unwrap();
	let err = build_vars_with_env_files_strict(dir.path(), &["bad.env".to_string()]).unwrap_err();
	assert!(matches!(err, crate::error::ComposeError::EnvFile(_)));
}

#[test]
fn first_disallowed_control_char_allows_tab_newline_cr() {
	// Legitimate dotenv escapes / multi-line values must pass.
	assert_eq!(first_disallowed_control_char("plain value"), None);
	assert_eq!(first_disallowed_control_char("a\tb\nc\rd"), None);
	// NUL, ESC and other C0/C1 controls are rejected.
	assert_eq!(first_disallowed_control_char("a\0b"), Some('\0'));
	assert_eq!(first_disallowed_control_char("x\x1by"), Some('\x1b'));
}

#[test]
fn strict_build_vars_errors_on_control_char_value_naming_file_and_key() {
	let dir = tempfile::tempdir().unwrap();
	// A NUL in an env-file value is rejected at load time, before any compose
	// parse, with an error that names the originating env file and key, not a
	// misattributed parse offset into the post-substitution document (#885).
	std::fs::write(dir.path().join("bad.env"), "SECRET=ab\0cd\n").unwrap();
	let err = build_vars_with_env_files_strict(dir.path(), &["bad.env".to_string()]).unwrap_err();
	assert!(
		matches!(err, crate::error::ComposeError::EnvFile(_)),
		"expected EnvFile error, got {err:?}"
	);
	let msg = err.to_string();
	assert!(msg.contains("bad.env"), "names the env file: {msg}");
	assert!(msg.contains("SECRET"), "names the offending key: {msg}");
	assert!(
		msg.contains("control"),
		"explains the control-char cause: {msg}"
	);
}

#[test]
fn strict_build_vars_allows_multiline_and_tab_values() {
	let dir = tempfile::tempdir().unwrap();
	// A multi-line quoted value (real newline) and a `\t` escape are legitimate
	// and must not be rejected by the control-char guard.
	std::fs::write(
		dir.path().join("ok.env"),
		"MULTI=\"line one\nline two\"\nTABBED=\"a\\tb\"\n",
	)
	.unwrap();
	let vars = build_vars_with_env_files_strict(dir.path(), &["ok.env".to_string()]).unwrap();
	assert_eq!(
		vars.get("MULTI").map(String::as_str),
		Some("line one\nline two")
	);
	assert_eq!(vars.get("TABBED").map(String::as_str), Some("a\tb"));
}
