use super::*;

#[cfg(unix)]
#[test]
fn a_refusal_names_the_directory_without_letting_it_drive_the_terminal() {
	use std::os::unix::fs::PermissionsExt;
	let root = tempfile::tempdir().unwrap();
	// An escape sequence in a path component: it comes from XDG_CONFIG_HOME.
	let dir = root.path().join("cfg\x1b[2J");
	std::fs::create_dir(&dir).unwrap();
	std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
	let err = require_private(&dir).unwrap_err().to_string();
	assert!(err.contains("writable by others"), "{err}");
	assert!(!err.chars().any(char::is_control), "{err:?}");
}
