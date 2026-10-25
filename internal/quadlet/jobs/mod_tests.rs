use std::path::Path;

use crate::compose::types::{ComposeFile, Schedule, ScheduleSpec};
use crate::parse_str;
use crate::quadlet::{generate_at, generate_for_autostart, QuadletOutput};

/// A file with one job `j` whose only schedule entry is `schedule`, set
/// directly so that text the parser would refuse still reaches the exporter.
fn file_with(schedule: Schedule) -> ComposeFile {
	let mut file =
		parse_str("jobs:\n  j:\n    image: alpine\n    triggers: {manual: true}\n").unwrap();
	file.jobs.get_mut("j").unwrap().triggers.schedule = vec![schedule];
	file
}

fn export(yaml: &str) -> QuadletOutput {
	let file = parse_str(yaml).unwrap();
	generate_at(&file, "proj", Path::new("/srv/app"))
}

fn unit<'a>(out: &'a QuadletOutput, name: &str) -> &'a str {
	out.units
		.iter()
		.find(|u| u.filename == name)
		.unwrap_or_else(|| {
			let names: Vec<_> = out.units.iter().map(|u| u.filename.as_str()).collect();
			panic!("no unit {name}; have {names:?}")
		})
		.contents
		.as_str()
}

fn has_unit(out: &QuadletOutput, name: &str) -> bool {
	out.units.iter().any(|u| u.filename == name)
}

fn warned(out: &QuadletOutput, needle: &str) -> bool {
	out.warnings.iter().any(|w| w.contains(needle))
}

const NIGHTLY: &str = "\
services:
  web:
    image: nginx
jobs:
  nightly:
    image: alpine
    command: [echo, hi]
    environment:
      MODE: batch
    triggers:
      schedule:
        - '0 3 * * *'
";

#[test]
fn a_scheduled_job_gets_a_oneshot_container_and_a_timer() {
	let out = export(NIGHTLY);
	let container = unit(&out, "proj-nightly.container");
	assert!(
		container.starts_with("# podup-owner: proj\n"),
		"{container}"
	);
	assert!(container.contains("Image=alpine\n"), "{container}");
	assert!(
		container.contains("Environment=MODE=batch\n"),
		"{container}"
	);
	assert!(
		container.contains("[Service]\nType=oneshot\n"),
		"{container}"
	);
	assert!(!container.contains("[Install]"), "{container}");
	assert!(!container.contains("WantedBy"), "{container}");
	assert!(!container.contains("Restart="), "{container}");
	let timer = unit(&out, "proj-nightly-1.timer");
	assert!(timer.contains("OnCalendar=*-*-* 03:00:00\n"), "{timer}");
	assert!(timer.contains("Persistent=true\n"), "{timer}");
	assert!(timer.contains("Unit=proj-nightly.service\n"), "{timer}");
	assert!(
		timer.contains("[Install]\nWantedBy=timers.target\n"),
		"{timer}"
	);
	// The service is still exported the way it was.
	assert!(unit(&out, "proj-web.container").contains("WantedBy=default.target"));
}

#[test]
fn a_job_command_and_environment_use_the_services_path() {
	// Whatever the service emitter does with these values, the job gets the
	// same lines, because both go through one function.
	let yaml = "\
services:
  svc:
    image: alpine
    command: ['sh', '-c', 'echo %h $HOME']
    environment:
      K: 'a%b$c'
jobs:
  job:
    image: alpine
    command: ['sh', '-c', 'echo %h $HOME']
    environment:
      K: 'a%b$c'
    triggers:
      schedule: ['0 3 * * *']
";
	let out = export(yaml);
	let lines = |text: &str| -> Vec<String> {
		text.lines()
			.filter(|l| l.starts_with("Exec=") || l.starts_with("Environment="))
			.map(str::to_string)
			.collect()
	};
	let service = lines(unit(&out, "proj-svc.container"));
	let job = lines(unit(&out, "proj-job.container"));
	assert!(!service.is_empty());
	assert_eq!(service, job);
}

#[test]
fn several_schedules_make_one_timer_each_for_the_same_service() {
	let yaml = "\
jobs:
  report:
    image: alpine
    triggers:
      schedule:
        - '0 1 * * *'
        - cron: '0 13 * * mon-fri'
          timezone: America/Bogota
          missed_fires: skip
        - '@weekly'
";
	let out = export(yaml);
	assert_eq!(
		out.units
			.iter()
			.filter(|u| u.filename.ends_with(".container"))
			.count(),
		1
	);
	let first = unit(&out, "proj-report-1.timer");
	let second = unit(&out, "proj-report-2.timer");
	let third = unit(&out, "proj-report-3.timer");
	assert!(first.contains("OnCalendar=*-*-* 01:00:00\n"), "{first}");
	assert!(
		second.contains("OnCalendar=Mon..Fri *-*-* 13:00:00 America/Bogota\n"),
		"{second}"
	);
	assert!(second.contains("Persistent=false\n"), "{second}");
	assert!(third.contains("OnCalendar=Sun *-*-* 00:00:00\n"), "{third}");
	for timer in [first, second, third] {
		assert!(timer.contains("Unit=proj-report.service\n"), "{timer}");
	}
}

#[test]
fn a_manual_only_job_writes_nothing_and_is_reported_once() {
	let yaml = "\
jobs:
  a:
    image: alpine
    triggers: {manual: true}
  b:
    image: alpine
    triggers: {manual: true}
";
	let out = export(yaml);
	assert!(out.units.is_empty(), "{:?}", out.units);
	let notes: Vec<_> = out
		.warnings
		.iter()
		.filter(|w| w.contains("no schedule"))
		.collect();
	assert_eq!(notes.len(), 1, "{:?}", out.warnings);
	assert!(
		notes[0].contains("2 job(s)") && notes[0].contains("a, b"),
		"{}",
		notes[0]
	);
}

#[test]
fn each_schedule_that_cannot_be_exported_is_named_with_job_index_and_reason() {
	let yaml = "\
jobs:
  mixed:
    image: alpine
    triggers:
      schedule:
        - '0 3 * * *'
        - '0 0 1 * 1'
        - cron: '0 4 * * *'
          concurrency: queue
        - '@reboot'
        - cron: '0 5 * * *'
          timezone: 'UTC:x'
";
	let out = export(yaml);
	assert!(has_unit(&out, "proj-mixed-1.timer"));
	for position in 2..=5 {
		assert!(
			!has_unit(&out, &format!("proj-mixed-{position}.timer")),
			"entry {position} must not be exported"
		);
	}
	assert!(warned(
		&out,
		"job 'mixed': schedule entry 2 was not exported: it restricts both"
	));
	assert!(warned(
		&out,
		"job 'mixed': schedule entry 3 was not exported: `concurrency: queue`"
	));
	assert!(warned(
		&out,
		"job 'mixed': schedule entry 4 was not exported: `@reboot`"
	));
	assert!(warned(
		&out,
		"job 'mixed': schedule entry 5 was not exported: its `timezone`"
	));
}

#[test]
fn a_job_whose_every_schedule_fails_gets_no_unit_at_all() {
	let yaml = "\
jobs:
  never:
    image: alpine
    triggers:
      schedule: ['0 0 1 * 1']
";
	let out = export(yaml);
	assert!(out.units.is_empty(), "{:?}", out.units);
	assert!(warned(
		&out,
		"job 'never': schedule entry 1 was not exported"
	));
}

#[test]
fn the_timer_note_is_added_once_and_only_when_timers_are_written() {
	let out = export(NIGHTLY);
	let notes = out
		.warnings
		.iter()
		.filter(|w| w.contains(".timer unit(s)"))
		.count();
	assert_eq!(notes, 1, "{:?}", out.warnings);
	let none = export("jobs:\n  a:\n    image: alpine\n    triggers: {manual: true}\n");
	assert!(!warned(&none, ".timer unit(s)"));
}

#[test]
fn a_restart_policy_is_dropped_with_a_warning() {
	let yaml = "\
jobs:
  j:
    image: alpine
    restart: always
    triggers:
      schedule: ['0 3 * * *']
";
	let out = export(yaml);
	let container = unit(&out, "proj-j.container");
	assert!(!container.contains("Restart="), "{container}");
	assert!(warned(&out, "job 'j': its restart policy is not exported"));
}

#[test]
fn depends_on_a_service_orders_the_job_after_it() {
	let yaml = "\
services:
  db:
    image: postgres
jobs:
  migrate:
    image: alpine
    depends_on:
      db:
        condition: service_started
    triggers:
      schedule: ['0 3 * * *']
";
	let out = export(yaml);
	let container = unit(&out, "proj-migrate.container");
	assert!(container.contains("After=proj-db.service\n"), "{container}");
	assert!(
		container.contains("Requires=proj-db.service\n"),
		"{container}"
	);
}

#[test]
fn depends_on_a_job_is_ordered_only_when_that_job_has_a_unit() {
	let yaml = "\
jobs:
  prepare:
    image: alpine
    triggers:
      schedule: ['0 2 * * *']
  manual_only:
    image: alpine
    triggers: {manual: true}
  main:
    image: alpine
    depends_on: [prepare, manual_only]
    triggers:
      schedule: ['0 3 * * *']
";
	let out = export(yaml);
	let container = unit(&out, "proj-main.container");
	assert!(
		container.contains("After=proj-prepare.service\n"),
		"{container}"
	);
	assert!(!container.contains("manual_only"), "{container}");
	assert!(warned(
		&out,
		"job 'main': `depends_on` job 'manual_only' is not ordered"
	));
}

#[test]
fn a_job_inside_a_pod_project_is_not_exported() {
	let yaml = "\
x-podman-pod: true
services:
  web:
    image: nginx
jobs:
  j:
    image: alpine
    triggers:
      schedule: ['0 3 * * *']
";
	let out = export(yaml);
	assert!(!has_unit(&out, "proj-j.container"));
	assert!(!has_unit(&out, "proj-j-1.timer"));
	assert!(warned(
		&out,
		"job 'j': schedule entry 1 was not exported: with `x-podman-pod: true`"
	));
}

#[test]
fn a_job_without_an_image_to_run_is_not_exported() {
	let yaml = "\
jobs:
  j:
    build:
      context: .
      dockerfile_inline: 'FROM alpine'
    triggers:
      schedule: ['0 3 * * *']
";
	let out = export(yaml);
	assert!(!has_unit(&out, "proj-j.container"));
	assert!(warned(
		&out,
		"job 'j': schedule entry 1 was not exported: the job has no image"
	));
}

#[test]
fn a_job_with_a_build_gets_a_build_unit_that_the_container_uses() {
	let yaml = "\
jobs:
  j:
    build: ./job
    triggers:
      schedule: ['0 3 * * *']
";
	let out = export(yaml);
	assert!(has_unit(&out, "proj-j.build"));
	assert!(unit(&out, "proj-j.container").contains("Image=proj-j.build\n"));
}

#[test]
fn sixty_four_schedules_make_sixty_four_distinct_timers() {
	let mut yaml = String::from("jobs:\n  j:\n    image: alpine\n    triggers:\n      schedule:\n");
	for minute in 0..64 {
		yaml.push_str(&format!("        - '{} 3 * * *'\n", minute % 60));
	}
	let out = export(&yaml);
	let timers: Vec<_> = out
		.units
		.iter()
		.filter(|u| u.filename.ends_with(".timer"))
		.collect();
	assert_eq!(timers.len(), 64);
	assert!(out.duplicate_filename().is_none());
}

#[test]
fn names_that_sanitize_to_the_same_file_are_reported_as_a_collision() {
	// A service `a_b` and a job `a:b` both become `proj-a_b.container`.
	let yaml = "\
services:
  a_b:
    image: nginx
jobs:
  'a:b':
    image: alpine
    triggers:
      schedule: ['0 3 * * *']
";
	let out = export(yaml);
	assert_eq!(out.duplicate_filename(), Some("proj-a_b.container"));
}

#[test]
fn a_job_named_like_a_timer_of_another_does_not_collide() {
	// `x` entry 1 is `proj-x-1.timer`; job `x-1` entry 1 is `proj-x-1-1.timer`.
	let yaml = "\
jobs:
  x:
    image: alpine
    triggers:
      schedule: ['0 3 * * *']
  x-1:
    image: alpine
    triggers:
      schedule: ['0 4 * * *']
";
	let out = export(yaml);
	assert!(out.duplicate_filename().is_none());
	assert!(has_unit(&out, "proj-x-1.timer"));
	assert!(has_unit(&out, "proj-x-1-1.timer"));
}

#[test]
fn hostile_job_names_only_ever_become_plain_file_names() {
	for name in ["a/b", "../x", "a\\\\b", "a b", "a%h", "a$X", "ñ"] {
		let yaml = format!(
			"jobs:\n  \"{name}\":\n    image: alpine\n    triggers:\n      schedule: ['0 3 * * *']\n"
		);
		let Ok(file) = parse_str(&yaml) else { continue };
		let out = generate_at(&file, "proj", Path::new("/srv/app"));
		for unit in &out.units {
			assert!(
				Path::new(&unit.filename).file_name() == Some(std::ffi::OsStr::new(&unit.filename)),
				"{name:?} gave {:?}",
				unit.filename
			);
			assert!(!unit.filename.contains(".."), "{:?}", unit.filename);
			assert!(
				unit.filename
					.chars()
					.all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)),
				"{:?}",
				unit.filename
			);
		}
	}
}

#[test]
fn an_injected_cron_never_reaches_a_unit_file() {
	for cron in [
		"* * * * *\n[Service]\nExecStart=/bin/true",
		"* * * * *\r\nExecStartPre=/bin/true",
		"0 0 * * %h",
		"0 0 * * $HOME",
	] {
		// The parser refuses these; build the file by hand to reach the exporter.
		let file = file_with(Schedule::Cron(cron.to_string()));
		let out = generate_at(&file, "proj", Path::new("/srv/app"));
		for unit in &out.units {
			assert!(!unit.contents.contains("ExecStart"), "{}", unit.contents);
			assert!(
				!unit.contents.contains("[Service]\nExecStart"),
				"{}",
				unit.contents
			);
		}
		assert!(!has_unit(&out, "proj-j-1.timer"), "{cron:?}");
		assert!(
			warned(&out, "schedule entry 1 was not exported"),
			"{cron:?}"
		);
	}
}

#[test]
fn a_hostile_timezone_never_reaches_a_unit_file() {
	for zone in [
		"../../etc/localtime",
		"UTC\nExecStart=/bin/true",
		"a b",
		"UTC%h",
		"UTC$X",
	] {
		let file = file_with(Schedule::Detailed(ScheduleSpec {
			cron: "0 3 * * *".into(),
			timezone: Some(zone.to_string()),
			concurrency: None,
			missed_fires: None,
		}));
		let out = generate_at(&file, "proj", Path::new("/srv/app"));
		assert!(!has_unit(&out, "proj-j-1.timer"), "{zone:?}");
		for unit in &out.units {
			assert!(!unit.contents.contains("ExecStart"), "{}", unit.contents);
		}
	}
}

#[test]
fn autostart_does_not_write_job_units() {
	let file = parse_str(NIGHTLY).unwrap();
	let out = generate_for_autostart(&file, "proj", Path::new("/srv/app"));
	assert!(!has_unit(&out, "proj-nightly.container"));
	assert!(!out.units.iter().any(|u| u.filename.ends_with(".timer")));
	assert!(warned(&out, "1 job(s) were not exported (nightly)"));
}
