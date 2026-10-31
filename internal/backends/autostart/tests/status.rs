use super::*;

#[test]
fn linger_off_produces_warning() {
	let mut sc = FakeCtl::new();
	sc.linger = "no".to_string();
	temp_env::with_var("USER", Some("tester"), || {
		assert!(linger_warning(&sc).is_some());
	});
}

#[test]
fn linger_on_produces_no_warning() {
	let sc = FakeCtl::new(); // linger = "yes"
	temp_env::with_var("USER", Some("tester"), || {
		assert!(linger_warning(&sc).is_none());
	});
}

#[test]
fn missing_runtime_dir_produces_warning() {
	temp_env::with_var_unset("XDG_RUNTIME_DIR", || {
		assert!(runtime_dir_warning().is_some());
	});
	temp_env::with_var("XDG_RUNTIME_DIR", Some("/run/user/1000"), || {
		assert!(runtime_dir_warning().is_none());
	});
}

#[test]
fn collect_status_reports_installed_and_state() {
	with_env(|root| {
		let sc = FakeCtl::new();
		install(&sc, &opts(root, "app", false, true)).unwrap();
		let r = collect_status(&sc, "app");
		assert!(r.unit_exists);
		assert!(r.unit_mode.is_some());
		assert_eq!(r.is_active, "active");
		assert_eq!(r.is_enabled, "enabled");
		assert!(r.linger);
		assert!(r.runtime_dir);
	});
}

#[test]
fn collect_status_reports_absent_unit() {
	with_env(|_root| {
		let sc = FakeCtl::new();
		let r = collect_status(&sc, "nope");
		assert!(!r.unit_exists);
		assert!(r.unit_mode.is_none());
	});
}

/// A project installed in quadlet mode has one unit per service, not the
/// single `podup-<project>.service` the status used to ask about. The status
/// has to find the install and ask about the units the project owns.
#[test]
fn collect_status_sees_a_quadlet_install() {
	with_env(|root| {
		let qdir = root.join("containers/systemd");
		std::fs::create_dir_all(&qdir).unwrap();
		std::fs::write(
			qdir.join("proj-web.container"),
			"# podup-owner: proj\n[Container]\nImage=nginx\n",
		)
		.unwrap();
		std::fs::write(
			qdir.join("proj-web.network"),
			"# podup-owner: proj\n[Network]\n",
		)
		.unwrap();

		let sc = FakeCtl::new();
		let r = collect_status(&sc, "proj");
		assert!(
			r.unit_exists,
			"a quadlet install must be reported as installed"
		);
		assert_eq!(r.unit_count, 2, "both own units must be counted");
		assert_eq!(r.is_active, "active");
		assert_eq!(r.is_enabled, "enabled");
		let calls = sc.systemctl_log();
		assert!(
			calls
				.iter()
				.any(|c| c.contains(&"proj-web.service".to_string())),
			"the status must ask about the project's own services: {calls:?}"
		);
		assert!(
			!calls
				.iter()
				.any(|c| c.contains(&"podup-proj.service".to_string())),
			"the status must not ask about a unit the project does not have: {calls:?}"
		);
	});
}

/// A unit of another project is not this project's install: the ownership
/// marker decides, the same way it does for the uninstall.
#[test]
fn collect_status_ignores_a_quadlet_unit_of_another_project() {
	with_env(|root| {
		let qdir = root.join("containers/systemd");
		std::fs::create_dir_all(&qdir).unwrap();
		std::fs::write(
			qdir.join("proj-web.container"),
			"# podup-owner: other\n[Container]\nImage=nginx\n",
		)
		.unwrap();

		let sc = FakeCtl::new();
		let r = collect_status(&sc, "proj");
		assert!(
			!r.unit_exists,
			"a unit that belongs to another project is not an install"
		);
	});
}

// --- the network shim (#1616's ordering is only real if the unit loads) ---

/// The trap, pinned. Real `systemctl show` exits **0** for a unit that does not
/// exist, printing `LoadState=not-found`. A guard reading the exit code would
/// call the shim present here and be wrong in exactly the case it exists to
/// catch, which is the same vacuous shape as the assertion #1616 replaced.
#[test]
fn a_missing_shim_is_read_from_load_state_not_from_the_exit_code() {
	let mut sc = FakeCtl::new();
	sc.show_load_state = "LoadState=not-found\n".to_string();
	let r = collect_status(&sc, "app");
	assert_eq!(r.network_wait, NetworkWait::NotFound);
	// The call really did succeed, so nothing about the status distinguished
	// these two cases except the string.
	assert_eq!(
		sc.systemctl(&["show", NETWORK_SHIM, "-p", "LoadState"])
			.unwrap()
			.status
			.code(),
		Some(0)
	);
}

#[test]
fn a_loaded_shim_reads_as_loaded() {
	let sc = FakeCtl::new();
	assert_eq!(collect_status(&sc, "app").network_wait, NetworkWait::Loaded);
}

/// "We could not tell" is not "it is not there". Collapsing the two would report
/// a broken ordering on every machine whose systemctl answered oddly.
#[test]
fn an_unrecognised_answer_is_unknown_rather_than_missing() {
	let mut sc = FakeCtl::new();
	sc.show_load_state = "LoadState=masked\n".to_string();
	assert!(matches!(
		collect_status(&sc, "app").network_wait,
		NetworkWait::Unknown(s) if s == "masked"
	));

	let mut sc = FakeCtl::new();
	sc.show_load_state = String::new();
	assert!(matches!(
		collect_status(&sc, "app").network_wait,
		NetworkWait::Unknown(_)
	));
}

/// The status asks about the shim by the same name the units order against, so
/// a rename cannot leave the guard checking a unit nothing depends on.
#[test]
fn the_status_asks_about_the_unit_the_units_actually_name() {
	let sc = FakeCtl::new();
	let _ = collect_status(&sc, "app");
	assert!(
		sc.systemctl_log()
			.iter()
			.any(|c| c.first().map(String::as_str) == Some("show")
				&& c.contains(&NETWORK_SHIM.to_string())),
		"{:?}",
		sc.systemctl_log()
	);
	// Every renderer that emits the ordering, not just the one that had it
	// first. Checking only service mode would leave start mode free to name a
	// different unit than the status reports on, which is the same silent
	// mismatch this whole check exists to surface.
	let service = render_service_unit(&ServiceUnitOpts::new(
		PathBuf::from("/usr/bin/podup"),
		vec![PathBuf::from("/srv/app/compose.yml")],
		"app".to_string(),
		PathBuf::from("/srv/app"),
	));
	let start = render_start_unit(&StartUnitOpts::new(
		PathBuf::from("/usr/bin/podman"),
		"app".to_string(),
		"app-web-1".to_string(),
	));
	for (mode, unit) in [("service", &service), ("start", &start)] {
		assert!(
			unit.contains(NETWORK_SHIM),
			"{mode} mode must order against the same name the status checks:\n{unit}"
		);
	}
}
