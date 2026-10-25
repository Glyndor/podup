//! Hostile and degenerate input to the job export: names and schedule text
//! that must never reach a unit file as anything but a plain name or a refusal.

use std::path::Path;

use super::tests::{export, file_with, has_unit, warned};
use crate::compose::types::{Schedule, ScheduleSpec};
use crate::parse_str;
use crate::quadlet::generate_at;

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
	let mut parsed = 0;
	for name in ["a/b", "../x", "a\\\\b", "a b", "a%h", "a$X", "ñ"] {
		let yaml = format!(
			"jobs:\n  \"{name}\":\n    image: alpine\n    triggers:\n      schedule: ['0 3 * * *']\n"
		);
		let Ok(file) = parse_str(&yaml) else { continue };
		parsed += 1;
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
	assert!(parsed >= 5, "the names must parse so that the export runs");
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
