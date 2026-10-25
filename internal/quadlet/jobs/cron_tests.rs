use super::*;
use crate::quadlet::jobs::on_calendar::render;

/// The OnCalendar text of a cron line that must be exportable.
fn calendar(expression: &str) -> String {
	render(&parse_cron(expression).unwrap_or_else(|e| panic!("{expression:?}: {e}")))
}

#[test]
fn exportable_expressions_and_their_calendar_text() {
	for (cron, expected) in [
		("0 3 * * *", "*-*-* 03:00:00"),
		("30 4 1 * *", "*-*-01 04:30:00"),
		("*/5 * * * *", "*-*-* *:00/5:00"),
		("* * * * *", "*-*-* *:*:00"),
		("0 */2 * * *", "*-*-* 00/2:00:00"),
		("0 9-17 * * 1-5", "Mon..Fri *-*-* 09..17:00:00"),
		("0 0 * * 0", "Sun *-*-* 00:00:00"),
		("0 0 * * 7", "Sun *-*-* 00:00:00"),
		("0 0 * * sat,sun", "Sat,Sun *-*-* 00:00:00"),
		("0 0 * * MON", "Mon *-*-* 00:00:00"),
		("15 14 1 jan,jul *", "*-01,07-01 14:15:00"),
		("0 0 1 */3 *", "*-01/3-01 00:00:00"),
		("0,30 8,20 * * *", "*-*-* 08,20:00,30:00"),
		("0 0 ? * 1", "Mon *-*-* 00:00:00"),
		("0 0 15 * ?", "*-*-15 00:00:00"),
		("0 12 * * 0-6", "*-*-* 12:00:00"),
		("0 12 * * 1,3,5", "Mon,Wed,Fri *-*-* 12:00:00"),
		("  0   3  *  *  *  ", "*-*-* 03:00:00"),
		("@hourly", "*-*-* *:00:00"),
		("@daily", "*-*-* 00:00:00"),
		("@weekly", "Sun *-*-* 00:00:00"),
		("@monthly", "*-*-01 00:00:00"),
		("@yearly", "*-01-01 00:00:00"),
		("@annually", "*-01-01 00:00:00"),
	] {
		assert_eq!(calendar(cron), expected, "cron {cron:?}");
	}
}

#[test]
fn a_restricted_day_of_month_and_day_of_week_is_not_exportable() {
	for cron in [
		"0 0 1 * 1",
		"0 0 1,15 * mon-fri",
		"0 0 13 * 5",
		"0 0 */2 * 1",
		"0 0 1-31 * 1",
		"0 0 1 * 1-7",
	] {
		let reason = parse_cron(cron).expect_err(cron);
		assert!(reason.contains("either"), "{cron}: {reason}");
	}
}

#[test]
fn constructs_outside_the_subset_are_not_exportable() {
	for cron in [
		"@reboot",
		"@midnight",
		"@every 5m",
		"@DAILY",
		"@",
		"@daily extra",
		"0 0 * *",
		"0 0 * * * *",
		"0 0 0 * * * *",
		"*",
		"0 0 L * *",
		"0 0 LW * *",
		"0 0 15W * *",
		"0 0 * * 5#2",
		"0 0 * * 5L",
		"0 0 ? * ?x",
		"0 ? * * *",
		"? * * * *",
		"60 * * * *",
		"* 24 * * *",
		"* * 0 * *",
		"* * * 13 *",
		"* * * * 8",
		"*/0 * * * *",
		"5/15 * * * *",
		"5-1 * * * *",
		"1,,2 * * * *",
		"0 0 * * * # comment",
		"0 0 * * mon-sun",
		"0 0 * feb-jan *",
		"0 0 * * foo",
		"",
	] {
		assert!(parse_cron(cron).is_err(), "{cron:?} must not be exported");
	}
}

#[test]
fn field_count_errors_name_the_count() {
	let seconds = parse_cron("0 0 0 * * *").unwrap_err();
	assert!(seconds.contains("seconds"), "{seconds}");
	let few = parse_cron("0 0 * *").unwrap_err();
	assert!(few.contains("4 fields"), "{few}");
}

#[test]
fn text_outside_the_cron_alphabet_is_refused_before_anything_else() {
	for cron in [
		"* * * * *\n[Service]\nExecStart=/bin/true",
		"* * * * *\r\n",
		"0 0 * * *\t",
		"0 0 * * %h",
		"0 0 * * $HOME",
		"0 0 * * *;rm",
		"0 0 * * \u{ff0a}",
		"0 0 * * *\u{2028}",
		"0 0 * * *\0",
		"0 0 * * \"1\"",
		"0 0 * * \\",
	] {
		let reason = parse_cron(cron).unwrap_err();
		assert!(reason.contains("character"), "{cron:?}: {reason}");
	}
}

#[test]
fn macro_and_five_field_forms_agree() {
	for (alias, fields) in [
		("@hourly", "0 * * * *"),
		("@daily", "0 0 * * *"),
		("@weekly", "0 0 * * 0"),
		("@monthly", "0 0 1 * *"),
		("@yearly", "0 0 1 1 *"),
		("@annually", "0 0 1 1 *"),
	] {
		assert_eq!(parse_cron(alias), parse_cron(fields), "{alias}");
	}
}
