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
