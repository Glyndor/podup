use super::*;

// --- quoting of the values in the unit ------------------------------------

#[test]
fn quotes_paths_with_spaces() {
	let mut o = opts_single();
	o.exe = PathBuf::from("/opt/my tools/podup");
	o.compose_files = vec![PathBuf::from("/srv/my app/compose.yml")];
	o.working_dir = PathBuf::from("/srv/my app");
	let s = render_service_unit(&o);
	// The exe and the compose path are double-quoted as single arguments.
	assert!(
		s.contains("ExecStart=\"/opt/my tools/podup\" -f \"/srv/my app/compose.yml\" -p app up -d")
	);
	// WorkingDirectory takes the rest of the line literally, so it is not quoted.
	assert!(s.contains("WorkingDirectory=/srv/my app"));
}

#[test]
fn bare_safe_accepts_paths_rejects_spaces() {
	assert!(is_bare_safe("/srv/app/compose.yml"));
	assert!(is_bare_safe("app-1_v2.0"));
	assert!(!is_bare_safe("has space"));
	assert!(!is_bare_safe(""));
	assert!(!is_bare_safe("a\"b"));
}

#[test]
fn quote_arg_escapes_quotes_and_backslashes() {
	assert_eq!(quote_arg("a b"), "\"a b\"");
	assert_eq!(quote_arg("a\"b"), "\"a\\\"b\"");
	assert_eq!(quote_arg("a\\b"), "\"a\\\\b\"");
}

// --- bug: `%`-specifiers are not escaped in unit values (systemd expands
// `%h`/`%i`/`%o`/... in every unit-file value, exec tokens and
// `WorkingDirectory=` alike; a literal `%` must be doubled to `%%` or a path
// like `/srv/50%off` gets `%o`-expanded, mis-resolving or failing to start). ---

#[test]
fn quote_arg_doubles_percent_even_in_a_bare_looking_token() {
	// `50%off` has no space/quote/control byte; the only reason it must be
	// quoted at all is the `%`, and the `%` itself must be doubled so systemd's
	// specifier expansion collapses it back to one literal `%` instead of
	// trying to expand `%o` as a specifier.
	assert_eq!(quote_arg("50%off"), "\"50%%off\"");
	assert_eq!(quote_arg("100%"), "\"100%%\"");
}

#[test]
fn render_service_unit_escapes_percent_in_working_directory() {
	let mut o = opts_single();
	o.working_dir = PathBuf::from("/srv/50%off");
	let s = render_service_unit(&o);
	assert!(
		s.contains("WorkingDirectory=/srv/50%%off"),
		"WorkingDirectory must double a literal '%' so systemd does not expand \
		 '%o' as a specifier:\n{s}"
	);
	assert!(!s.contains("WorkingDirectory=/srv/50%off\n"), "{s}");
}

#[test]
fn render_service_unit_escapes_percent_in_exec_line_tokens() {
	let mut o = opts_single();
	o.compose_files = vec![PathBuf::from("/srv/50%off/docker-compose.yml")];
	let s = render_service_unit(&o);
	assert!(
		s.contains("50%%off/docker-compose.yml"),
		"an exec-line token containing '%' must render it doubled as '%%':\n{s}"
	);
	assert!(!s.contains("50%off/docker-compose.yml"), "{s}");
}

#[test]
fn render_service_unit_normal_path_round_trips_unchanged() {
	// A path with no '%' must not be touched by the escaping fix.
	let s = render_service_unit(&opts_single());
	assert!(s.contains("WorkingDirectory=/srv/app"));
	assert!(
		s.contains("ExecStart=/usr/local/bin/podup -f /srv/app/docker-compose.yml -p app up -d")
	);
}

#[test]
fn render_service_unit_escapes_percent_in_project_description() {
	// `Description=` interpolates `opts.project` directly; a literal `%` in
	// it must be doubled exactly like every other in-unit value, so systemd's
	// specifier expansion does not treat e.g. `%h` as a specifier. This holds
	// regardless of the external `is_safe_project_name` gate: the module's
	// own %-invariant should not depend on it.
	let mut o = opts_single();
	o.project = "50%h".to_string();
	let s = render_service_unit(&o);
	assert!(
		s.contains("Description=podup 50%%h"),
		"Description= must double a literal '%' in the project name:\n{s}"
	);
	assert!(!s.contains("Description=podup 50%h\n"), "{s}");
}
