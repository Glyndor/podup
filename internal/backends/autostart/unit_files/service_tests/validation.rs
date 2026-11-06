use super::*;

// --- what the unit accepts -----------------------------------------------

/// A socket the unit cannot carry is refused where the operator is watching,
/// not at boot with nothing to say why.
#[test]
fn a_socket_the_unit_cannot_carry_is_refused() {
	let mut o = opts_single();
	o.socket = Some("tcp://example.invalid:2375".to_string());
	let err = validate_unit_opts(&o).expect_err("a remote socket must be refused");
	assert!(err.contains("local socket"), "{err}");
	assert!(validate_unit_opts(&opts_single()).is_ok());
}

#[test]
fn validate_rejects_control_chars_in_workdir() {
	let mut o = opts_single();
	o.working_dir = PathBuf::from("/srv/app\nExecStartPre=/bin/evil");
	let err = validate_unit_opts(&o).unwrap_err();
	assert!(err.contains("working directory"), "{err}");
}

#[test]
fn validate_rejects_control_chars_in_exe_and_files() {
	let mut o = opts_single();
	o.exe = PathBuf::from("/usr/local/bin/pod\x07up");
	assert!(validate_unit_opts(&o).is_err());

	let mut o = opts_single();
	o.compose_files = vec![PathBuf::from("/srv/app/com\npose.yml")];
	assert!(validate_unit_opts(&o).is_err());

	let mut o = opts_single();
	o.env_files = vec!["/srv/app/.env\r".to_string()];
	assert!(validate_unit_opts(&o).is_err());
}

#[test]
fn validate_accepts_normal_paths() {
	assert!(validate_unit_opts(&opts_single()).is_ok());
	let mut o = opts_single();
	o.working_dir = PathBuf::from("/srv/my app (prod)");
	assert!(validate_unit_opts(&o).is_ok());
}

#[test]
fn validate_accepts_percent_in_paths() {
	// A literal '%' is a legitimate path/flag character (e.g. `/srv/50%off`);
	// it must be escaped at render time, never rejected at validation time.
	let mut o = opts_single();
	o.working_dir = PathBuf::from("/srv/50%off");
	o.compose_files = vec![PathBuf::from("/srv/50%off/docker-compose.yml")];
	assert!(validate_unit_opts(&o).is_ok());
}
