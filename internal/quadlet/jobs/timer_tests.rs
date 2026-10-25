use super::*;

fn spec<'a>(job: &'a str, calendar: &'a str, timezone: Option<&'a str>) -> TimerSpec<'a> {
	TimerSpec {
		project: "proj",
		job,
		index: 2,
		calendar,
		timezone,
		persistent: true,
	}
}

#[test]
fn a_timer_has_the_calendar_the_zone_the_target_and_the_install_section() {
	let unit = timer_unit(&spec("nightly", "*-*-* 03:00:00", Some("Europe/Madrid"))).unwrap();
	assert_eq!(unit.filename, "proj-nightly-2.timer");
	assert_eq!(
		unit.contents,
		"# podup-owner: proj\n\
		 [Unit]\n\
		 Description=proj-nightly schedule 2 (podup)\n\
		 \n\
		 [Timer]\n\
		 OnCalendar=*-*-* 03:00:00 Europe/Madrid\n\
		 Persistent=true\n\
		 Unit=proj-nightly.service\n\
		 \n\
		 [Install]\n\
		 WantedBy=timers.target\n"
	);
}

#[test]
fn missed_fires_skip_is_persistent_false() {
	let mut skip = spec("j", "*-*-* 03:00:00", None);
	skip.persistent = false;
	let unit = timer_unit(&skip).unwrap();
	assert!(unit.contents.contains("Persistent=false\n"));
	assert!(unit.contents.contains("OnCalendar=*-*-* 03:00:00\n"));
}

#[test]
fn a_timezone_outside_the_whitelist_is_refused() {
	for zone in [
		"",
		"..",
		"../etc/passwd",
		"Europe/../Madrid",
		"/etc/localtime",
		"-Europe",
		"Europe/",
		"Europe//Madrid",
		"Europe/Mad rid",
		"Europe/Madrid ",
		"Europe/Madrid\n[Service]\nExecStart=/bin/true",
		"UTC\r",
		"UTC\0",
		"UTC\t",
		"UTC%h",
		"UTC$X",
		"Europe/Mádrid",
		"UTC;",
		"UTC:x",
		"UTC.1",
		"UTC\"",
		"UTC\\",
		&"A".repeat(MAX_TIMEZONE_BYTES + 1),
	] {
		assert!(!is_safe_timezone(zone), "{zone:?} must be refused");
		let result = timer_unit(&spec("j", "*-*-* 03:00:00", Some(zone)));
		assert!(result.is_err(), "{zone:?} must not reach a unit");
	}
	for zone in [
		"UTC",
		"Europe/Madrid",
		"America/Argentina/Buenos_Aires",
		"Etc/GMT+5",
		"Etc/GMT-14",
	] {
		assert!(is_safe_timezone(zone), "{zone}");
	}
}

#[test]
fn a_calendar_that_fails_the_final_check_never_reaches_a_unit() {
	for calendar in [
		"*-*-* 00:00:00\n[Service]\nExecStart=/bin/true",
		"*-*-* 00:00:%h",
		"",
		"*-*-* 00:00:00\0",
	] {
		let result = timer_unit(&spec("j", calendar, None));
		assert!(result.is_err(), "{calendar:?}");
	}
}

#[test]
fn unit_names_are_plain_components() {
	for good in ["proj-job.container", "a_b.c-d.timer", "x"] {
		assert!(is_safe_unit_name(good), "{good}");
	}
	for bad in [
		"",
		".hidden",
		"-flag",
		"a..b",
		"a/b",
		"a\\b",
		"a\0b",
		"a b",
		"a%b",
		"a$b",
		"a@b",
		"a\nb",
		"ñ",
		&"a".repeat(MAX_UNIT_NAME_BYTES + 1),
	] {
		assert!(!is_safe_unit_name(bad), "{bad:?}");
	}
	assert!(is_safe_unit_name(&"a".repeat(MAX_UNIT_NAME_BYTES)));
}

#[test]
fn hostile_job_names_become_plain_file_names() {
	for job in [
		"../../etc/cron.d/x",
		"a/b",
		"a\\b",
		"a b",
		"a\nb",
		"a%h",
		"a$X",
		"nul\0byte",
		"ñandú",
		"..",
		".",
		"-rf",
	] {
		if let Ok(unit) = timer_unit(&spec(job, "*-*-* 03:00:00", None)) {
			assert!(
				is_safe_unit_name(&unit.filename),
				"{job:?} gave {:?}",
				unit.filename
			);
			assert!(!unit.filename.contains(['/', '\\', '\0']));
			// The target service is built from the same stem.
			let target = unit
				.contents
				.lines()
				.find_map(|line| line.strip_prefix("Unit="))
				.unwrap();
			assert!(is_safe_unit_name(target), "{target}");
			assert_eq!(
				unit.contents.lines().filter(|l| l.starts_with('[')).count(),
				3
			);
		}
	}
}

#[test]
fn a_control_character_in_the_project_is_refused() {
	let mut bad = spec("j", "*-*-* 03:00:00", None);
	bad.project = "p\n[Service]";
	assert!(timer_unit(&bad).is_err());
}

#[test]
fn a_name_that_is_too_long_is_refused() {
	let long = "j".repeat(MAX_UNIT_NAME_BYTES);
	assert!(timer_unit(&spec(&long, "*-*-* 03:00:00", None)).is_err());
}
