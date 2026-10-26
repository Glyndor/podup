use super::*;

fn set(values: &[u8]) -> BTreeSet<u8> {
	values.iter().copied().collect()
}

#[test]
fn star_is_every_value() {
	assert_eq!(parse_field("*", &HOUR).unwrap().len(), 24);
	assert_eq!(parse_field("*", &MONTH).unwrap().len(), 12);
	assert_eq!(parse_field("*", &DAY_OF_MONTH).unwrap().len(), 31);
}

#[test]
fn lists_ranges_and_steps_expand_to_sets() {
	assert_eq!(parse_field("5", &MINUTE).unwrap(), set(&[5]));
	assert_eq!(parse_field("1,15,30", &MINUTE).unwrap(), set(&[1, 15, 30]));
	assert_eq!(parse_field("9-12", &HOUR).unwrap(), set(&[9, 10, 11, 12]));
	assert_eq!(parse_field("*/15", &MINUTE).unwrap(), set(&[0, 15, 30, 45]));
	assert_eq!(
		parse_field("10-30/10", &MINUTE).unwrap(),
		set(&[10, 20, 30])
	);
	// Day of month starts at 1, so `*/2` is the odd days.
	let odd = parse_field("*/2", &DAY_OF_MONTH).unwrap();
	assert_eq!(odd.iter().copied().next(), Some(1));
	assert_eq!(odd.len(), 16);
	assert_eq!(
		parse_field("1-3,10,20-21", &DAY_OF_MONTH).unwrap(),
		set(&[1, 2, 3, 10, 20, 21])
	);
}

#[test]
fn names_are_case_insensitive_and_work_in_ranges() {
	assert_eq!(parse_field("JAN", &MONTH).unwrap(), set(&[1]));
	assert_eq!(parse_field("dec", &MONTH).unwrap(), set(&[12]));
	assert_eq!(
		parse_field("mon-fri", &DAY_OF_WEEK).unwrap(),
		set(&[1, 2, 3, 4, 5])
	);
	assert_eq!(parse_field("Sun", &DAY_OF_WEEK).unwrap(), set(&[0]));
	assert_eq!(parse_field("jun-aug", &MONTH).unwrap(), set(&[6, 7, 8]));
}

#[test]
fn question_mark_is_a_wildcard_only_for_the_day_fields() {
	assert_eq!(parse_field("?", &DAY_OF_MONTH).unwrap().len(), 31);
	assert_eq!(parse_field("?", &DAY_OF_WEEK).unwrap().len(), 8);
	assert!(parse_field("?", &MINUTE).is_err());
	assert!(parse_field("?", &MONTH).is_err());
	// Inside a list or with a step it is a Quartz construct.
	assert!(parse_field("?,1", &DAY_OF_MONTH).is_err());
	assert!(parse_field("?/2", &DAY_OF_MONTH).is_err());
}

#[test]
fn constructs_outside_the_subset_are_rejected() {
	for (text, spec) in [
		("", &MINUTE),
		("60", &MINUTE),
		("24", &HOUR),
		("0", &DAY_OF_MONTH),
		("32", &DAY_OF_MONTH),
		("0", &MONTH),
		("13", &MONTH),
		("8", &DAY_OF_WEEK),
		("L", &DAY_OF_MONTH),
		("15W", &DAY_OF_MONTH),
		("5#2", &DAY_OF_WEEK),
		("1L", &DAY_OF_WEEK),
		("1,,2", &MINUTE),
		("1,", &MINUTE),
		(",1", &MINUTE),
		("5-1", &MINUTE),
		("1-2-3", &MINUTE),
		("*/0", &MINUTE),
		("*/60", &MINUTE),
		("*/", &MINUTE),
		("*/x", &MINUTE),
		("*/-1", &MINUTE),
		("5/15", &MINUTE),
		("-5", &MINUTE),
		("+5", &MINUTE),
		("5.0", &MINUTE),
		("005", &MINUTE),
		(" 5", &MINUTE),
		("foo", &MONTH),
		("mon", &MONTH),
		("mon-sun", &DAY_OF_WEEK),
		("janu", &MONTH),
		("1-*", &MINUTE),
		("*-5", &MINUTE),
		("**", &MINUTE),
	] {
		assert!(
			parse_field(text, spec).is_err(),
			"{text:?} in {} must be rejected",
			spec.label
		);
	}
}

#[test]
fn the_day_of_week_accepts_seven_as_sunday_in_the_raw_set() {
	assert_eq!(parse_field("7", &DAY_OF_WEEK).unwrap(), set(&[7]));
	assert_eq!(parse_field("5-7", &DAY_OF_WEEK).unwrap(), set(&[5, 6, 7]));
}

#[test]
fn only_a_bare_star_or_question_mark_is_unrestricted() {
	assert!(is_unrestricted("*"));
	assert!(is_unrestricted("?"));
	assert!(!is_unrestricted("*/2"));
	assert!(!is_unrestricted("1-31"));
	assert!(!is_unrestricted("1"));
}
