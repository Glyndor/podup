//! Tests for the `autostart` command group: the wiring between the CLI's
//! flags and the units it generates.
//!
//! Split out of the command module to keep it within the source line limit
//! and to follow the repository's rule that a test module under `internal/`
//! lives in a sibling `<stem>_tests.rs` file.

use super::*;

/// The CLI's own values, with the socket the operator selected.
fn env(socket: Option<&str>) -> AutostartEnv<'static> {
	AutostartEnv {
		profile: &[],
		env_files: &[],
		socket: socket.map(str::to_string),
		connection_pool_size: None,
		project_directory: None,
	}
}

/// The CLI's own values, with the project directory the operator selected.
fn env_with_dir(dir: PathBuf) -> AutostartEnv<'static> {
	AutostartEnv {
		profile: &[],
		env_files: &[],
		socket: None,
		connection_pool_size: None,
		project_directory: Some(dir),
	}
}

fn compose_file() -> ComposeFile {
	podup::parse_str("services:\n  web:\n    image: nginx\n").expect("the file parses")
}

/// The socket the operator selected must reach the generated unit: the
/// unit runs with no environment of the operator's, so without it the boot
/// path would talk to the default instance instead of the selected one.
#[test]
fn the_selected_socket_reaches_the_service_unit() {
	let dir = tempfile::tempdir().expect("a temp dir");
	let compose = dir.path().join("compose.yml");
	std::fs::write(&compose, "services:\n  web:\n    image: nginx\n").expect("the file");
	let opts = service_unit_opts(
		&env(Some("/run/user/1000/podman/podman.sock")),
		&[compose],
		"app",
		dir.path(),
		&compose_file(),
	)
	.expect("the options build");
	let unit = podup::autostart::render_service_unit(&opts);
	assert!(
		unit.contains("--socket /run/user/1000/podman/podman.sock"),
		"the selected socket must travel into the unit:\n{unit}"
	);
}

/// The same for start mode, whose unit runs `podman` directly.
#[test]
fn the_selected_socket_reaches_the_start_unit() {
	let opts = start_unit_opts(
		&env(Some("/run/user/1000/podman/podman.sock")),
		"app",
		"app-web-1",
		PathBuf::from("/usr/bin/podman"),
		&compose_file(),
	);
	let unit = podup::autostart::render_start_unit(&opts);
	assert!(
		unit.contains("--url unix:///run/user/1000/podman/podman.sock"),
		"the selected socket must travel into the unit:\n{unit}"
	);
}

/// With no socket selected, the unit carries none: the boot path then
/// talks to the default instance, exactly as the operator's own run would.
#[test]
fn no_selected_socket_leaves_the_unit_without_one() {
	let opts = start_unit_opts(
		&env(None),
		"app",
		"app-web-1",
		PathBuf::from("/usr/bin/podman"),
		&compose_file(),
	);
	let unit = podup::autostart::render_start_unit(&opts);
	assert!(!unit.contains("--url"), "{unit}");
}

/// The project directory must reach the unit: podup resolves relative
/// paths against the compose file's own directory, so without the flag
/// the boot path resolves them somewhere else than the operator's own run
/// did.
#[test]
fn the_selected_project_directory_reaches_the_service_unit() {
	let dir = tempfile::tempdir().expect("a temp dir");
	let compose = dir.path().join("compose.yml");
	std::fs::write(&compose, "services:\n  web:\n    image: nginx\n").expect("the file");
	let opts = service_unit_opts(
		&env_with_dir(dir.path().to_path_buf()),
		&[compose],
		"app",
		dir.path(),
		&compose_file(),
	)
	.expect("the options build");
	let unit = podup::autostart::render_service_unit(&opts);
	// The path is written as the platform spells it once resolved (macOS maps
	// `/var` to `/private/var`, Windows writes a quoted verbatim path), so the
	// test looks for the flag and for the directory's own name.
	let name = dir
		.path()
		.file_name()
		.and_then(|n| n.to_str())
		.expect("a temp dir has a name");
	let flag_line = unit
		.lines()
		.find(|l| l.starts_with("ExecStart=") && l.contains("--project-directory"))
		.unwrap_or_else(|| panic!("no ExecStart carries the flag:\n{unit}"));
	assert!(
		flag_line.contains(name),
		"the selected project directory must travel into the unit:\n{unit}"
	);
}

/// Without one selected there is nothing to carry: podup derives the base
/// from the compose file, as it does for the operator's own run.
#[test]
fn no_selected_project_directory_leaves_the_unit_without_one() {
	let dir = tempfile::tempdir().expect("a temp dir");
	let compose = dir.path().join("compose.yml");
	std::fs::write(&compose, "services:\n  web:\n    image: nginx\n").expect("the file");
	let opts = service_unit_opts(&env(None), &[compose], "app", dir.path(), &compose_file())
		.expect("the options build");
	let unit = podup::autostart::render_service_unit(&opts);
	assert!(!unit.contains("--project-directory"), "{unit}");
}

/// A compose file with a service behind an inactive profile.
fn profiled_file() -> ComposeFile {
	podup::parse_str(
		"services:\n  web:\n    image: nginx\n  debug:\n    image: nginx\n    profiles: [debug]\n",
	)
	.expect("the file parses")
}

/// Quadlet mode writes one unit per service, so a service behind an
/// inactive profile would get a unit of its own and start at every boot.
#[test]
fn the_active_profiles_select_the_services_the_quadlet_units_are_built_from() {
	let selected = selected_services(&profiled_file(), &[]);
	let units =
		podup::quadlet::generate_for_autostart(&selected, "proj", std::path::Path::new("/srv/app"));
	let names: Vec<&str> = units.units.iter().map(|u| u.filename.as_str()).collect();
	assert!(
		names.contains(&"proj-web.container"),
		"the service without a profile must have its unit: {names:?}"
	);
	assert!(
		!names.contains(&"proj-debug.container"),
		"the service behind an inactive profile must not have a unit: {names:?}"
	);
}

/// Start mode resolves the one container the project has. A project whose
/// other service sits behind an inactive profile still has exactly one
/// active service, and must not be refused for having two.
#[test]
fn the_active_profiles_select_the_container_the_start_unit_names() {
	let selected = selected_services(&profiled_file(), &[]);
	let container = podup::autostart::sole_container(&selected, "proj")
		.expect("the project has one active service");
	assert_eq!(container, "proj-web-1");
}
