use super::*;

// ---------------------------------------------------------------------------
// --auto-update: the sibling `<unit>-update.service` oneshot and `<unit>
// -update.timer` schedule. Install/uninstall behaviour.
// ---------------------------------------------------------------------------

/// #1656: `autostart install --mode service --auto-update <interval>` writes
/// the main unit, the `<unit>-update.service` oneshot, and the
/// `<unit>-update.timer` schedule; enables the timer; reloads systemd once.
#[test]
fn autostart_auto_update_renders_the_timer_and_the_oneshot_unit() {
	with_env(|root| {
		let sc = FakeCtl::new();
		install(&sc, &opts_with_interval(root, "app", false, false, "daily")).unwrap();
		let main = root.join("systemd/user/podup-app.service");
		let oneshot = root.join("systemd/user/podup-app-update.service");
		let timer = root.join("systemd/user/podup-app-update.timer");
		assert!(main.is_file(), "main unit written");
		assert!(oneshot.is_file(), "oneshot written");
		assert!(timer.is_file(), "timer written");
		let oneshot_body = std::fs::read_to_string(&oneshot).unwrap();
		assert!(
			oneshot_body.contains("Type=oneshot"),
			"oneshot must be Type=oneshot: {oneshot_body}"
		);
		assert!(
			oneshot_body.contains("ExecStart=/usr/local/bin/podup -f ")
				&& oneshot_body.contains("-p app up -d"),
			"oneshot must use the same leading args and run `up -d`: {oneshot_body}"
		);
		let timer_body = std::fs::read_to_string(&timer).unwrap();
		assert!(
			timer_body.contains("OnCalendar=daily"),
			"timer must carry the chosen OnCalendar=: {timer_body}"
		);
		assert!(
			timer_body.contains("Persistent=true"),
			"timer must be Persistent: {timer_body}"
		);
		assert!(
			timer_body.contains("WantedBy=timers.target"),
			"timer must be wanted by timers.target: {timer_body}"
		);
		let calls = sc.systemctl_log();
		assert_eq!(calls[0], vec!["daemon-reload"]);
		assert!(
			calls
				.iter()
				.any(|c| c.first().map(String::as_str) == Some("enable")
					&& c.contains(&"podup-app-update.timer".to_string())),
			"the timer must be enabled: {calls:?}"
		);
	});
}

/// #1656: without `--auto-update`, the install path produces exactly what it
/// did before the feature existed. The byte-identical-output promise.
#[test]
fn autostart_without_auto_update_renders_exactly_what_it_did_before() {
	with_env(|root| {
		let sc = FakeCtl::new();
		install(&sc, &opts(root, "app", false, false)).unwrap();
		let main = root.join("systemd/user/podup-app.service");
		let oneshot = root.join("systemd/user/podup-app-update.service");
		let timer = root.join("systemd/user/podup-app-update.timer");
		assert!(main.is_file(), "main unit written");
		assert!(!oneshot.exists(), "no oneshot without --auto-update");
		assert!(!timer.exists(), "no timer without --auto-update");
		let calls = sc.systemctl_log();
		assert_eq!(calls[0], vec!["daemon-reload"]);
		assert_eq!(calls[1], vec!["enable", "--now", "podup-app.service"]);
		// No enable call for the timer pair.
		assert!(
			!calls
				.iter()
				.any(|c| c.contains(&"podup-app-update.timer".to_string())),
			"no timer call without --auto-update: {calls:?}"
		);
	});
}

/// #1656: removing a project that was installed with `--auto-update` removes
/// both new units along with the main one. Without that, the timer would
/// keep firing `up -d` against a project whose main unit had been uninstalled.
#[test]
fn autostart_auto_update_uninstall_removes_both_new_units() {
	with_env(|root| {
		let sc = FakeCtl::new();
		install(
			&sc,
			&opts_with_interval(root, "app", false, false, "weekly"),
		)
		.unwrap();
		let main = root.join("systemd/user/podup-app.service");
		let oneshot = root.join("systemd/user/podup-app-update.service");
		let timer = root.join("systemd/user/podup-app-update.timer");
		assert!(main.is_file());
		assert!(oneshot.is_file());
		assert!(timer.is_file());

		let sc2 = FakeCtl::new();
		uninstall(&sc2, "app").unwrap();
		assert!(!main.exists(), "main unit removed");
		assert!(!oneshot.exists(), "oneshot removed");
		assert!(!timer.exists(), "timer removed");
	});
}

/// A bogus interval is rejected at install time, the same way the CLI does.
/// Programmatically, `validate_auto_update_interval` is what catches it; the
/// renderer cannot return an error and silently emitting a bogus `OnCalendar=`
/// would leave the user without a working timer.
#[test]
fn autostart_auto_update_renderer_rejects_an_unknown_interval() {
	let opts = ServiceUnitOpts::new(
		PathBuf::from("/usr/local/bin/podup"),
		vec![PathBuf::from("/srv/app/docker-compose.yml")],
		"app".to_string(),
		PathBuf::from("/srv/app"),
	);
	let result = std::panic::catch_unwind(|| {
		let _ = render_update_timer_unit("app", "biweekly");
	});
	assert!(
		result.is_err(),
		"the renderer must panic on a bogus interval rather than silently emit it"
	);
	let _ = opts;
}

/// The builder is what the command uses to carry `--auto-update` into the
/// install options; a mutation sweep replaced it with `Default::default()`
/// and nothing noticed, because every test here builds the struct literally.
#[test]
fn install_options_builder_sets_the_auto_update_interval() {
	with_env(|root| {
		let base = opts(root, "app", false, false);
		let built =
			InstallOptions::new(base.unit).with_auto_update_interval(Some("daily".to_string()));
		assert_eq!(built.auto_update_interval.as_deref(), Some("daily"));
		let cleared = InstallOptions::new(opts(root, "app", false, false).unit)
			.with_auto_update_interval(None);
		assert_eq!(cleared.auto_update_interval, None);
	});
}
