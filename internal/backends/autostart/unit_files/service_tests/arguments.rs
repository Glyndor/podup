use super::*;

// --- what travels into the command ---------------------------------------

/// The socket the operator selected must travel into the unit: the boot path
/// runs with no environment of the operator's, so without it `podup` would
/// talk to the default instance, which is a different one whenever
/// `--socket` pointed somewhere else.
#[test]
fn the_selected_socket_travels_into_the_command() {
	let mut o = opts_single();
	o.socket = Some("/run/user/1000/podman/podman.sock".to_string());
	let s = render_service_unit(&o);
	assert!(
		s.contains(
			"-p app --socket /run/user/1000/podman/podman.sock up -d --no-build --pull never"
		),
		"{s}"
	);
	assert!(
		s.contains("-p app --socket /run/user/1000/podman/podman.sock stop"),
		"{s}"
	);
}

/// The auto-update oneshot talks to the same instance as the boot unit, so
/// the socket travels into it too.
#[test]
fn the_selected_socket_travels_into_the_auto_update_command() {
	let mut o = opts_single();
	o.socket = Some("/run/user/1000/podman/podman.sock".to_string());
	let s = super::render_update_service_unit(&o);
	assert!(
		s.contains("-p app --socket /run/user/1000/podman/podman.sock up -d"),
		"{s}"
	);
}

/// The project directory must travel into the command: podup resolves
/// relative paths against the compose file's own directory, so without the
/// flag the boot path resolves them somewhere else than the operator's own
/// run did.
#[test]
fn the_selected_project_directory_travels_into_the_command() {
	let mut o = opts_single();
	o.project_directory = Some(PathBuf::from("/srv/data"));
	let s = render_service_unit(&o);
	assert!(
		s.contains("-p app --project-directory /srv/data up -d --no-build --pull never"),
		"{s}"
	);
	assert!(
		s.contains("-p app --project-directory /srv/data stop"),
		"{s}"
	);
}

#[test]
fn renders_multiple_files_in_order() {
	let mut o = opts_single();
	o.compose_files = vec![
		PathBuf::from("/srv/app/base.yml"),
		PathBuf::from("/srv/app/override.yml"),
	];
	let s = render_service_unit(&o);
	assert!(s.contains(
		"ExecStart=/usr/local/bin/podup -f /srv/app/base.yml -f /srv/app/override.yml -p app up -d --no-build --pull never"
	));
	assert!(s.contains(
		"ExecStop=/usr/local/bin/podup -f /srv/app/base.yml -f /srv/app/override.yml -p app stop"
	));
}

#[test]
fn includes_profiles_and_env_files() {
	let mut o = opts_single();
	o.profiles = vec!["prod".to_string(), "web".to_string()];
	o.env_files = vec!["/srv/app/.env.prod".to_string()];
	let s = render_service_unit(&o);
	assert!(s.contains(
		"-p app --profile prod --profile web --env-file /srv/app/.env.prod up -d --no-build --pull never"
	));
	assert!(s.contains("-p app --profile prod --profile web --env-file /srv/app/.env.prod stop"));
}
