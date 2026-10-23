use super::*;

#[test]
fn uninstall_disables_removes_and_reloads() {
	with_env(|root| {
		let sc = FakeCtl::new();
		install(&sc, &opts(root, "app", false, true)).unwrap();
		let path = root.join("systemd/user/podup-app.service");
		assert!(path.exists());

		let sc2 = FakeCtl::new();
		uninstall(&sc2, "app").unwrap();
		assert!(!path.exists(), "unit file removed");
		let calls = sc2.systemctl_log();
		// The `is-active` probe only asks whether systemd knows the unit at
		// all; anything but exit 4 means disable it. Two more probes for the
		// auto-update units happen even when none were installed, they
		// answer the same question, just for siblings that may or may not be
		// present.
		assert_eq!(calls[0], vec!["is-active", "--quiet", "podup-app.service"]);
		assert_eq!(calls[1], vec!["disable", "--now", "podup-app.service"]);
		// Two more probe+disable pairs for the auto-update units, even though
		// no install in this test wrote them, the uninstall path probes them
		// unconditionally so a half-installed pair still gets cleaned up.
		assert_eq!(
			calls[2],
			vec!["is-active", "--quiet", "podup-app-update.timer"]
		);
		assert_eq!(calls[3], vec!["disable", "--now", "podup-app-update.timer"]);
		assert_eq!(
			calls[4],
			vec!["is-active", "--quiet", "podup-app-update.service"]
		);
		assert_eq!(
			calls[5],
			vec!["disable", "--now", "podup-app-update.service"]
		);
		assert_eq!(calls[6], vec!["daemon-reload"]);
	});
}

/// #1080: a `disable --now` that fails on an installed unit was swallowed by
/// `let _ =`, so uninstall exited 0 with the service still enabled and
/// running. Measured against real systemd: with the unit file present,
/// `disable --now` exits 0 whether or not the unit was ever enabled or
/// started, so a non-zero exit here is always a real failure.
#[test]
fn uninstall_reports_a_failed_disable() {
	with_env(|root| {
		install(&FakeCtl::new(), &opts(root, "app", false, true)).unwrap();
		let path = root.join("systemd/user/podup-app.service");
		assert!(path.exists());

		let mut sc = FakeCtl::new();
		sc.fail = true;
		let err = uninstall(&sc, "app")
			.expect_err("a failed disable must not be reported as a clean uninstall");
		assert!(matches!(err, ComposeError::Autostart(_)), "got {err:?}");
		assert!(err.to_string().contains("disable"), "got {err}");
	});
}

/// Uninstalling when systemd has never heard of the unit stays a silent
/// no-op. `is-active` exit 4 ("no such unit") is the signal, not the unit
/// file, which is a poor proxy: a fragment deleted out of band leaves the
/// unit loaded, enabled and running, and only `disable --now` clears it.
#[test]
fn uninstall_runs_no_disable_when_systemd_does_not_know_the_unit() {
	with_env(|_root| {
		let mut sc = FakeCtl::new();
		sc.is_active_code = 4 << 8; // systemd: no such unit
		uninstall(&sc, "app").expect("uninstalling nothing is not a failure");
		let calls = sc.systemctl_log();
		assert!(
			!calls
				.iter()
				.any(|c| c.first().map(String::as_str) == Some("disable")),
			"nothing is installed, so nothing should be disabled: {calls:?}"
		);
		assert_eq!(calls.last().unwrap(), &vec!["daemon-reload".to_string()]);
	});
}

/// The mirror case, and the reason the file is not the gate: the unit file is
/// gone but systemd still has the unit loaded and running (a manual `rm`, a
/// restored `~/.config`, a half-finished uninstall). `disable --now` is the
/// only thing that clears that state, so it must still run.
#[test]
fn uninstall_disables_a_known_unit_whose_file_is_already_gone() {
	with_env(|root| {
		let path = root.join("systemd/user/podup-app.service");
		assert!(!path.exists());
		// Default `is_active_code` is 0: systemd knows the unit.
		let sc = FakeCtl::new();
		uninstall(&sc, "app").expect("uninstall must still succeed");
		let calls = sc.systemctl_log();
		assert!(
			calls.contains(&vec![
				"disable".to_string(),
				"--now".to_string(),
				"podup-app.service".to_string()
			]),
			"a unit systemd still knows must be disabled even with no file: {calls:?}"
		);
	});
}
