use super::*;

#[test]
fn container_services_names_only_containers() {
	let file = parse_str(IMG).unwrap();
	let units = quadlet::generate_at(&file, "proj", Path::new(BASE)).units;
	// The default network unit is present too, but only `.container`s become services.
	assert_eq!(
		container_services(&units),
		vec!["proj-web.service".to_string()]
	);
}

#[test]
fn install_writes_units_reloads_then_starts() {
	with_env(|root| {
		let sc = FakeCtl::new();
		install_quadlet(
			&sc,
			&parse_str(IMG).unwrap(),
			"proj",
			Path::new(BASE),
			false,
			false,
		)
		.unwrap();
		assert!(root.join("containers/systemd/proj-web.container").is_file());
		let calls = sc.log();
		assert_eq!(calls[0], vec!["daemon-reload"]);
		assert_eq!(calls[1], vec!["start", "proj-web.service"]);
	});
}

#[test]
fn no_start_reloads_but_starts_nothing() {
	with_env(|root| {
		let sc = FakeCtl::new();
		install_quadlet(
			&sc,
			&parse_str(IMG).unwrap(),
			"proj",
			Path::new(BASE),
			true,
			false,
		)
		.unwrap();
		assert!(root.join("containers/systemd/proj-web.container").is_file());
		assert_eq!(sc.log(), vec![vec!["daemon-reload".to_string()]]);
	});
}

/// #1747 (L6): three services would have caused three forks of
/// `systemctl --user start`. The fix bundles the whole project's services
/// onto one argv: `systemctl --user start svc-a.service svc-b.service
/// svc-c.service`. systemd serializes them whether they came in on one
/// argv or several, so the only practical change is the fork count. The
/// test asserts the bundled shape directly so a regression to the
/// per-service loop is caught.
#[test]
fn install_starts_every_service_in_a_single_systemctl_call() {
	with_env(|_root| {
		let sc = FakeCtl::new();
		install_quadlet(
			&sc,
			&parse_str(IMG_MULTI).unwrap(),
			"proj",
			Path::new(BASE),
			false,
			false,
		)
		.unwrap();
		let calls = sc.log();
		assert_eq!(
			calls[0],
			vec!["daemon-reload".to_string()],
			"the daemon-reload must still be its own call"
		);
		// One start call, with every service bundled onto the argv. N forks
		// of systemd would have meant N consecutive `["start", "<svc>"]`
		// entries in the call log.
		let starts: Vec<_> = calls
			.iter()
			.filter(|c| c.first().map(String::as_str) == Some("start"))
			.collect();
		assert_eq!(
			starts.len(),
			1,
			"every container service must share one `systemctl start` call, not one each: {calls:?}"
		);
		let start = starts[0];
		assert_eq!(start[0], "start", "the verb is `start`: {start:?}");
		assert!(
			start.len() >= 4
				&& start.contains(&"proj-web.service".to_string())
				&& start.contains(&"proj-api.service".to_string())
				&& start.contains(&"proj-worker.service".to_string()),
			"every generated service must appear as an argv operand: {start:?}"
		);
	});
}

#[test]
fn dry_run_writes_nothing_and_runs_no_systemctl() {
	with_env(|root| {
		let sc = FakeCtl::new();
		install_quadlet(
			&sc,
			&parse_str(IMG).unwrap(),
			"proj",
			Path::new(BASE),
			false,
			true,
		)
		.unwrap();
		assert!(!root.join("containers/systemd/proj-web.container").exists());
		assert!(sc.log().is_empty());
	});
}

#[test]
fn install_refuses_when_service_mode_is_present() {
	with_env(|root| {
		let sd = root.join("systemd/user");
		std::fs::create_dir_all(&sd).unwrap();
		std::fs::write(sd.join("podup-proj.service"), "x").unwrap();
		let err = install_quadlet(
			&FakeCtl::new(),
			&parse_str(IMG).unwrap(),
			"proj",
			Path::new(BASE),
			false,
			false,
		)
		.unwrap_err();
		assert!(format!("{err}").contains("service-mode autostart unit"));
	});
}

/// A service removed from the compose file must lose its unit when the
/// project is installed again. The unit keeps its container, which then
/// starts at every boot even though the project no longer has that service.
#[test]
fn reinstall_retires_the_units_of_services_the_compose_file_no_longer_has() {
	with_env(|root| {
		let two = "services:\n  web:\n    image: nginx\n  db:\n    image: postgres\n";
		install_quadlet(
			&FakeCtl::new(),
			&parse_str(two).unwrap(),
			"proj",
			Path::new(BASE),
			true,
			false,
		)
		.unwrap();
		let web = root.join("containers/systemd/proj-web.container");
		let db = root.join("containers/systemd/proj-db.container");
		assert!(web.is_file() && db.is_file());

		let sc = FakeCtl::new();
		install_quadlet(
			&sc,
			&parse_str(IMG).unwrap(),
			"proj",
			Path::new(BASE),
			true,
			false,
		)
		.unwrap();

		assert!(
			web.is_file(),
			"the unit of the service that stayed must remain"
		);
		assert!(
			!db.exists(),
			"the unit of the removed service must be retired"
		);
		assert!(
			sc.log()
				.contains(&vec!["stop".to_string(), "proj-db.service".to_string()]),
			"the retired service must be stopped: {:?}",
			sc.log()
		);
	});
}

/// Only the units this project owns are retired: a sibling project's unit
/// that shares the prefix is left alone, exactly as the uninstall leaves it.
#[test]
fn reinstall_does_not_retire_a_sibling_projects_unit() {
	with_env(|root| {
		install_quadlet(
			&FakeCtl::new(),
			&parse_str(IMG).unwrap(),
			"app",
			Path::new(BASE),
			true,
			false,
		)
		.unwrap();
		install_quadlet(
			&FakeCtl::new(),
			&parse_str(IMG).unwrap(),
			"app-extra",
			Path::new(BASE),
			true,
			false,
		)
		.unwrap();

		let sc = FakeCtl::new();
		install_quadlet(
			&sc,
			&parse_str("services:\n  api:\n    image: nginx\n").unwrap(),
			"app",
			Path::new(BASE),
			true,
			false,
		)
		.unwrap();

		assert!(
			root.join("containers/systemd/app-extra-web.container")
				.is_file(),
			"the sibling project's unit must survive"
		);
		assert!(
			!sc.log().contains(&vec![
				"stop".to_string(),
				"app-extra-web.service".to_string()
			]),
			"the sibling project's service must not be stopped: {:?}",
			sc.log()
		);
	});
}
