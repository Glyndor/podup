//! The `OnCalendar=` text is accepted by `systemd-analyze calendar`, and its
//! next fire times are the ones the cron expression gives.
//!
//! The oracle is a hand-written matcher per expression, not the converter's own
//! output, and it steps through time a minute at a time. The tests skip, with a
//! message, where `systemd-analyze` is not installed or has no `--base-time`.

use std::process::Command;

use crate::compose::types::Schedule;
use crate::quadlet::jobs::schedule::plan;

/// How many fire times are compared for each expression.
const FIRES: usize = 6;
/// 2026-03-01 00:00:00 UTC, a Sunday.
const BASE: &str = "2026-03-01 00:00:00";
const BASE_DAYS: i64 = 20_513;

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
	let year = if month <= 2 { year - 1 } else { year };
	let era = year.div_euclid(400);
	let yoe = year - era * 400;
	let mp = (month + 9) % 12;
	let doy = (153 * mp + 2) / 5 + day - 1;
	let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
	era * 146_097 + doe - 719_468
}

/// Civil date of a day count, inverse of [`days_from_civil`].
fn civil_from_days(days: i64) -> (i64, i64, i64) {
	let z = days + 719_468;
	let era = z.div_euclid(146_097);
	let doe = z - era * 146_097;
	let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
	let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
	let mp = (5 * doy + 2) / 153;
	let day = doy - (153 * mp + 2) / 5 + 1;
	let month = if mp < 10 { mp + 3 } else { mp - 9 };
	let year = yoe + era * 400 + i64::from(month <= 2);
	(year, month, day)
}

/// (minute, hour, day of month, month, day of week with Sunday 0)
type Fields = (i64, i64, i64, i64, i64);

/// The first `FIRES` minutes after the base time that `matches`.
fn oracle(matches: fn(Fields) -> bool, years: i64) -> Vec<String> {
	let mut found = Vec::new();
	// systemd reports the first elapse strictly after the base time.
	for minute_of_run in 1..(years * 366 * 24 * 60) {
		let days = BASE_DAYS + minute_of_run / 1440;
		let (hour, minute) = ((minute_of_run % 1440) / 60, minute_of_run % 60);
		let (year, month, day) = civil_from_days(days);
		let weekday = (days + 4).rem_euclid(7);
		if matches((minute, hour, day, month, weekday)) {
			found.push(format!(
				"{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:00"
			));
			if found.len() == FIRES {
				break;
			}
		}
	}
	found
}

/// `systemd-analyze calendar` for `text`: the fire times, or `None` when the tool
/// is missing or too old for `--base-time`. Panics when it rejects the text.
fn analyze(text: &str) -> Option<Vec<String>> {
	let output = Command::new("systemd-analyze")
		.args(["calendar", "--base-time", BASE, "--iterations"])
		.arg(FIRES.to_string())
		.arg(text)
		.env("TZ", "UTC")
		.env("LC_ALL", "C")
		.output()
		.ok()?;
	let stderr = String::from_utf8_lossy(&output.stderr);
	if stderr.contains("unrecognized option") || stderr.contains("Unknown option") {
		return None;
	}
	assert!(
		output.status.success(),
		"systemd-analyze rejected {text:?}: {stderr}"
	);
	let stdout = String::from_utf8_lossy(&output.stdout);
	Some(
		stdout
			.lines()
			.map(str::trim)
			.filter(|line| line.starts_with("Next elapse:") || line.starts_with("Iteration #"))
			.filter_map(|line| {
				line.split_whitespace()
					.collect::<Vec<_>>()
					.windows(2)
					.find(|pair| pair[0].len() == 10 && pair[0].as_bytes()[4] == b'-')
					.map(|pair| format!("{} {}", pair[0], pair[1]))
			})
			.collect(),
	)
}

fn check(cron: &str, matches: fn(Fields) -> bool) {
	check_for(cron, 6, matches);
}

fn check_for(cron: &str, years: i64, matches: fn(Fields) -> bool) {
	let planned = plan(&Schedule::Cron(cron.to_string())).unwrap();
	let Some(got) = analyze(&planned.calendar) else {
		eprintln!("skipped: systemd-analyze with --base-time is not available");
		return;
	};
	assert_eq!(
		got,
		oracle(matches, years),
		"cron {cron:?} became {:?}",
		planned.calendar
	);
}

#[test]
fn the_calendar_text_fires_when_the_cron_expression_does() {
	check("0 3 * * *", |(m, h, ..)| m == 0 && h == 3);
	check("*/5 * * * *", |(m, ..)| m % 5 == 0);
	check("0 */2 * * *", |(m, h, ..)| m == 0 && h % 2 == 0);
	check("30 4 1 * *", |(m, h, d, ..)| m == 30 && h == 4 && d == 1);
	check("0 9-17 * * 1-5", |(m, h, _, _, w)| {
		m == 0 && (9..=17).contains(&h) && (1..=5).contains(&w)
	});
	check("0 0 * * 0", |(m, h, _, _, w)| m == 0 && h == 0 && w == 0);
	check("0 0 * * 7", |(m, h, _, _, w)| m == 0 && h == 0 && w == 0);
	check("15 14 1 jan,jul *", |(m, h, d, mo, _)| {
		m == 15 && h == 14 && d == 1 && (mo == 1 || mo == 7)
	});
	check("0 0 1 */3 *", |(m, h, d, mo, _)| {
		m == 0 && h == 0 && d == 1 && mo % 3 == 1
	});
	check("0,30 8,20 * * *", |(m, h, ..)| {
		(m == 0 || m == 30) && (h == 8 || h == 20)
	});
	check("0 0 */2 * *", |(m, h, d, ..)| {
		m == 0 && h == 0 && d % 2 == 1
	});
	check("0 12 * * sat,sun", |(m, h, _, _, w)| {
		m == 0 && h == 12 && (w == 6 || w == 0)
	});
	check("5-59/9 * * * *", |(m, ..)| m >= 5 && (m - 5) % 9 == 0);
	check("0-30/10 8 * * *", |(m, h, ..)| {
		h == 8 && m <= 30 && m % 10 == 0
	});
	check("0 0 31 * *", |(m, h, d, ..)| m == 0 && h == 0 && d == 31);
	check_for("0 0 29 2 *", 26, |(m, h, d, mo, _)| {
		m == 0 && h == 0 && d == 29 && mo == 2
	});
	check("@hourly", |(m, ..)| m == 0);
	check("@weekly", |(m, h, _, _, w)| m == 0 && h == 0 && w == 0);
	check("@monthly", |(m, h, d, ..)| m == 0 && h == 0 && d == 1);
	check("@yearly", |(m, h, d, mo, _)| {
		m == 0 && h == 0 && d == 1 && mo == 1
	});
	check("0 0 * * 5-7", |(m, h, _, _, w)| {
		m == 0 && h == 0 && (w == 5 || w == 6 || w == 0)
	});
	check("*/20 1-3 10-12 * *", |(m, h, d, ..)| {
		m % 20 == 0 && (1..=3).contains(&h) && (10..=12).contains(&d)
	});
}

#[test]
fn every_calendar_text_with_a_time_zone_is_accepted_by_systemd() {
	for zone in [
		"UTC",
		"Europe/Madrid",
		"America/Argentina/Buenos_Aires",
		"Etc/GMT+5",
	] {
		let planned = plan(&Schedule::Detailed(crate::compose::types::ScheduleSpec {
			cron: "0 3 * * *".into(),
			timezone: Some(zone.into()),
			concurrency: None,
			missed_fires: None,
		}))
		.unwrap();
		let text = format!("{} {zone}", planned.calendar);
		match Command::new("systemd-analyze")
			.args(["calendar", &text])
			.output()
		{
			Ok(out) => assert!(
				out.status.success(),
				"{text:?}: {}",
				String::from_utf8_lossy(&out.stderr)
			),
			Err(_) => eprintln!("skipped: systemd-analyze is not installed"),
		}
	}
}

#[test]
fn the_oracle_agrees_with_the_known_base_date() {
	assert_eq!(days_from_civil(2026, 3, 1), BASE_DAYS);
	assert_eq!(civil_from_days(BASE_DAYS), (2026, 3, 1));
	assert_eq!((BASE_DAYS + 4).rem_euclid(7), 0, "2026-03-01 is a Sunday");
	assert_eq!(
		oracle(|(m, h, ..)| m == 0 && h == 0, 1)[0],
		"2026-03-02 00:00:00"
	);
}

/// The Quadlet generator turns the job's `.container` into a oneshot service
/// that removes its container, and ignores the `.timer` files next to it.
#[cfg(unix)]
#[test]
fn quadlet_turns_the_job_unit_into_a_oneshot_service() {
	let generator =
		std::path::Path::new("/usr/lib/systemd/system-generators/podman-system-generator");
	if !generator.exists() {
		eprintln!("skipped: podman-system-generator is not installed");
		return;
	}
	let file = crate::parse_str(
		"jobs:\n  nightly:\n    image: docker.io/library/alpine\n    command: [echo, hi]\n    triggers:\n      schedule: ['0 3 * * *']\n",
	)
	.unwrap();
	let out = crate::quadlet::generate_at(&file, "proj", std::path::Path::new("/srv/app"));
	let dir = tempfile::tempdir().unwrap();
	crate::quadlet::write_units(dir.path(), &out.units).unwrap();
	let run = Command::new(generator)
		.arg("-dryrun")
		.env_clear()
		.env("QUADLET_UNIT_DIRS", dir.path())
		.output()
		.unwrap();
	let text = String::from_utf8_lossy(&run.stdout);
	assert!(
		run.status.success(),
		"{}",
		String::from_utf8_lossy(&run.stderr)
	);
	assert!(text.contains("---proj-nightly.service---"), "{text}");
	assert!(text.contains("Type=oneshot"), "{text}");
	assert!(text.contains("--rm"), "{text}");
	assert!(!text.contains("sdnotify"), "{text}");
	assert!(!text.contains("WantedBy"), "{text}");
	assert!(
		!text.contains("timer"),
		"the generator reads no .timer: {text}"
	);
}
