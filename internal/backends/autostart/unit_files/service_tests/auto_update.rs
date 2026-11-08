// ---------------------------------------------------------------------------
// --auto-update: the sibling `<unit>.update.service` oneshot and `<unit>
// .update.timer` schedule (render-only; install/uninstall are exercised in
// `tests.rs`).
// ---------------------------------------------------------------------------

fn opts_single_for_timer() -> super::ServiceUnitOpts {
	super::ServiceUnitOpts {
		exe: std::path::PathBuf::from("/usr/local/bin/podup"),
		compose_files: vec![std::path::PathBuf::from("/srv/app/docker-compose.yml")],
		project: "app".to_string(),
		working_dir: std::path::PathBuf::from("/srv/app"),
		profiles: Vec::new(),
		env_files: Vec::new(),
		max_stop_grace_secs: None,
		socket: None,
		project_directory: None,
	}
}

/// The auto-update oneshot unit carries the same leading arguments as the
/// main unit (so `-f`/`-p`/`--profile`/`--env-file` travel together) and ends
/// in `up -d`. Without the matching trailing arguments, an `--auto-update`
/// timer firing `podup up` against the wrong project would be the worst
/// possible kind of quiet failure.
#[test]
fn autostart_update_service_uses_same_leading_args_then_up_minus_d() {
	let opts = opts_single_for_timer();
	let s = super::render_update_service_unit(&opts);
	assert!(s.contains("Type=oneshot"), "{s}");
	assert!(
		s.contains("ExecStart=/usr/local/bin/podup -f /srv/app/docker-compose.yml -p app up -d"),
		"{s}"
	);
	assert!(s.contains("WorkingDirectory=/srv/app"), "{s}");
	// The timer is what gets enabled; a oneshot with its own [Install] could be
	// enabled on its own and fire at every login instead of on the schedule.
	assert!(!s.contains("[Install]"), "{s}");
}

/// The auto-update service is the one that refreshes images, so it must not
/// inherit the boot unit's `--no-build --pull never`.
#[test]
fn autostart_update_service_exec_start_is_pure_up_minus_d() {
	let opts = opts_single_for_timer();
	let s = super::render_update_service_unit(&opts);
	let exec_start_line = s
		.lines()
		.find(|l| l.starts_with("ExecStart="))
		.expect("ExecStart line is present");
	assert_eq!(
		exec_start_line,
		"ExecStart=/usr/local/bin/podup -f /srv/app/docker-compose.yml -p app up -d"
	);
}

/// The timer carries `OnCalendar=<word>`, `Persistent=true` (missed fires
/// catch up on next boot), and `WantedBy=timers.target`.
#[test]
fn autostart_update_timer_carries_on_calendar_persistent_and_timers_target() {
	for word in ["hourly", "daily", "weekly"] {
		let s = super::render_update_timer_unit("app", word);
		assert!(
			s.contains(&format!("OnCalendar={word}")),
			"word {word}: {s}"
		);
		assert!(s.contains("Persistent=true"), "{s}");
		assert!(s.contains("WantedBy=timers.target"), "{s}");
	}
}

/// An unknown interval is rejected, the same way the CLI does, but the
/// programmatic helper does it in `validate_auto_update_interval`, since the
/// unit-renderer cannot return an error and silently emitting a bogus
/// `OnCalendar=` would leave the user without a working timer.
#[test]
fn autostart_auto_update_rejects_an_unknown_interval() {
	let err = super::validate_auto_update_interval("biweekly")
		.expect_err("biweekly must not be accepted");
	assert!(
		err.contains("biweekly") && err.contains("hourly") && err.contains("weekly"),
		"the error must name the offending value and the three allowed spellings: {err}"
	);
}
