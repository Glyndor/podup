use super::*;
use crate::quadlet::jobs::cron::parse_cron;

fn text(cron: &str) -> String {
	render(&parse_cron(cron).unwrap())
}

#[test]
fn a_full_set_is_a_star_and_a_series_is_start_slash_step() {
	assert_eq!(text("*/10 * * * *"), "*-*-* *:00/10:00");
	assert_eq!(text("5-59/9 * * * *"), "*-*-* *:05/9:00");
	// Two values are a list, not a series.
	assert_eq!(text("0,30 * * * *"), "*-*-* *:00,30:00");
	// A series that stops short of the end of the range is spelled out.
	assert_eq!(text("0-30/10 * * * *"), "*-*-* *:00,10,20,30:00");
	// Uneven spacing is spelled out.
	assert_eq!(text("1,2,4 * * * *"), "*-*-* *:01,02,04:00");
}

#[test]
fn runs_of_three_or_more_become_ranges() {
	assert_eq!(text("1-3 * * * *"), "*-*-* *:01..03:00");
	assert_eq!(text("1-2 * * * *"), "*-*-* *:01,02:00");
	assert_eq!(text("0 0 1-5,10 * *"), "*-*-01..05,10 00:00:00");
	assert_eq!(text("0 0 * * 5-7"), "Fri..Sun *-*-* 00:00:00");
}

#[test]
fn weekdays_are_listed_monday_first_and_a_full_week_is_dropped() {
	assert_eq!(text("0 0 * * 0,1"), "Mon,Sun *-*-* 00:00:00");
	assert_eq!(text("0 0 * * 0-6"), "*-*-* 00:00:00");
	assert_eq!(text("0 0 * * 1-7"), "*-*-* 00:00:00");
}

#[test]
fn what_render_produces_passes_the_final_check() {
	for cron in [
		"* * * * *",
		"*/7 */3 */5 */2 *",
		"0 0 * * 1,3,5",
		"59 23 31 12 *",
		"@yearly",
		"0-59 0-23 1-31 1-12 *",
	] {
		let rendered = text(cron);
		assert!(is_safe_calendar(&rendered), "{cron}: {rendered:?}");
	}
}

#[test]
fn the_final_check_refuses_anything_a_calendar_does_not_contain() {
	for hostile in [
		"",
		" ",
		" *-*-* 00:00:00",
		"*-*-* 00:00:00 ",
		"*-*-*  00:00:00",
		"*-*-* 00:00:00\n[Service]\nExecStart=/bin/true",
		"*-*-* 00:00:00\r",
		"*-*-* 00:00:00\0",
		"*-*-* 00:00:00\t",
		"*-*-* 00:00:%h",
		"*-*-* 00:00:$X",
		"*-*-* 00:00:00;",
		"*-*-* 00:00:00\"",
		"*-*-* 00:00:00\\",
		"*-*-* \u{ff10}0:00:00",
		"*-*-* 00:00:00\u{202e}",
		"*-*-* 00:00:00 # x",
		"*-*-* 00:00:00=",
		"*-*-* 00:00:00@",
	] {
		assert!(!is_safe_calendar(hostile), "{hostile:?} must be refused");
	}
	let too_long = "1".repeat(MAX_CALENDAR_BYTES + 1);
	assert!(!is_safe_calendar(&too_long));
}
