use super::*;

// --- char predicates ---

#[test]
fn var_start_and_char_classes() {
	assert!(is_var_start('_'));
	assert!(is_var_start('a'));
	assert!(!is_var_start('1'));
	assert!(!is_var_start('-'));
	assert!(is_var_char('9'));
	assert!(!is_var_char('-'));
}

// --- collect_var_name ---

#[test]
fn collect_var_name_stops_at_non_var_char() {
	let mut it = peekable("NAME-rest");
	assert_eq!(collect_var_name(&mut it), "NAME");
	// The `-` and everything after it is left unconsumed.
	assert_eq!(it.collect::<String>(), "-rest");
}

// --- parse_braced_var: bare + every modifier form ---

#[test]
fn parse_bare_var_consumes_closing_brace() {
	let mut it = peekable("FOO}tail");
	let (name, modifier) = parse_braced_var(&mut it).unwrap();
	assert_eq!(name, "FOO");
	assert!(matches!(modifier, Modifier::None));
	assert_eq!(it.collect::<String>(), "tail");
}

#[test]
fn parse_unterminated_var_is_none_modifier() {
	let mut it = peekable("FOO");
	let (name, modifier) = parse_braced_var(&mut it).unwrap();
	assert_eq!(name, "FOO");
	assert!(matches!(modifier, Modifier::None));
}

#[test]
fn parse_each_modifier_form() {
	type Check = fn(&Modifier) -> bool;
	let cases: &[(&str, Check)] = &[
		(
			"V:-d}",
			|m| matches!(m, Modifier::DefaultIfUnsetOrEmpty(s) if s == "d"),
		),
		(
			"V-d}",
			|m| matches!(m, Modifier::DefaultIfUnset(s) if s == "d"),
		),
		(
			"V:+a}",
			|m| matches!(m, Modifier::AltIfSetAndNonEmpty(s) if s == "a"),
		),
		("V+a}", |m| matches!(m, Modifier::AltIfSet(s) if s == "a")),
		(
			"V:?e}",
			|m| matches!(m, Modifier::ErrorIfUnsetOrEmpty(s) if s == "e"),
		),
		(
			"V?e}",
			|m| matches!(m, Modifier::ErrorIfUnset(s) if s == "e"),
		),
	];
	for (input, check) in cases {
		let mut it = peekable(input);
		let (name, modifier) = parse_braced_var(&mut it).unwrap();
		assert_eq!(name, "V", "input {input}");
		assert!(check(&modifier), "modifier mismatch for {input}");
	}
}

#[test]
fn parse_colon_without_known_op_defaults_to_default_if_unset_or_empty() {
	let mut it = peekable("V:x}");
	let (_, modifier) = parse_braced_var(&mut it).unwrap();
	assert!(matches!(modifier, Modifier::DefaultIfUnsetOrEmpty(s) if s == "x"));
}

// --- parse_braced_var: name validation ---

#[test]
fn parse_valid_braced_var() {
	let mut it = peekable("FOO}");
	let (name, modifier) = parse_braced_var(&mut it).unwrap();
	assert_eq!(name, "FOO");
	assert!(matches!(modifier, Modifier::None));
}

#[test]
fn parse_valid_braced_var_with_default() {
	let mut it = peekable("FOO:-default}");
	let (name, modifier) = parse_braced_var(&mut it).unwrap();
	assert_eq!(name, "FOO");
	assert!(matches!(modifier, Modifier::DefaultIfUnsetOrEmpty(s) if s == "default"));
}

#[test]
fn parse_braced_var_rejects_space_in_name() {
	// `${FOO BAR}` must not produce a lookup key containing a space.
	let mut it = peekable("FOO BAR}");
	let err = parse_braced_var(&mut it).expect_err("space in name must be rejected");
	assert!(
		matches!(err, ComposeError::InvalidSubstitution(_)),
		"{err:?}"
	);
}

#[test]
fn parse_braced_var_rejects_dot_in_name() {
	// `${FOO.BAR}` must not produce a lookup key containing a dot.
	let mut it = peekable("FOO.BAR}");
	let err = parse_braced_var(&mut it).expect_err("dot in name must be rejected");
	assert!(
		matches!(err, ComposeError::InvalidSubstitution(_)),
		"{err:?}"
	);
}

#[test]
fn parse_braced_var_rejects_empty_name() {
	// `${}` has no name and must be rejected, not resolved to an empty string.
	let mut it = peekable("}");
	let err = parse_braced_var(&mut it).expect_err("empty name must be rejected");
	assert!(
		matches!(err, ComposeError::InvalidSubstitution(_)),
		"{err:?}"
	);
}

#[test]
fn parse_braced_var_rejects_digit_leading_name() {
	// `${1BAD}` is not a valid identifier (must start with a letter or `_`).
	let mut it = peekable("1BAD}");
	let err = parse_braced_var(&mut it).expect_err("digit-leading name must be rejected");
	assert!(
		matches!(err, ComposeError::InvalidSubstitution(_)),
		"{err:?}"
	);
}

#[test]
fn parse_braced_var_underscore_name_is_valid() {
	// A leading underscore is a valid identifier start.
	let mut it = peekable("_FOO}");
	let (name, modifier) = parse_braced_var(&mut it).unwrap();
	assert_eq!(name, "_FOO");
	assert!(matches!(modifier, Modifier::None));
}

#[test]
fn parse_unterminated_modifier_is_error() {
	// `${TAG:-latest` (no closing `}`) must not swallow the rest of the input as
	// the default value; it is reported as a malformed substitution.
	let mut it = peekable("TAG:-latest\nmore: data\n");
	let err = parse_braced_var(&mut it).expect_err("missing close brace must error");
	assert!(
		matches!(err, ComposeError::InvalidSubstitution(_)),
		"{err:?}"
	);
}
