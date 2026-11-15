use super::*;

// ${VAR:-default}

#[test]
fn default_if_unset_or_empty_when_unset() {
	assert_eq!(
		substitute("${X:-fallback}", &vars(&[])).unwrap(),
		"fallback"
	);
}

#[test]
fn default_if_unset_or_empty_when_empty() {
	assert_eq!(
		substitute("${X:-fallback}", &vars(&[("X", "")])).unwrap(),
		"fallback"
	);
}

#[test]
fn default_if_unset_or_empty_when_set() {
	assert_eq!(
		substitute("${X:-fallback}", &vars(&[("X", "real")])).unwrap(),
		"real"
	);
}

// ${VAR-default}

#[test]
fn default_if_unset_when_unset() {
	assert_eq!(substitute("${X-fallback}", &vars(&[])).unwrap(), "fallback");
}

#[test]
fn default_if_unset_when_empty_keeps_empty() {
	assert_eq!(
		substitute("${X-fallback}", &vars(&[("X", "")])).unwrap(),
		""
	);
}

#[test]
fn default_if_unset_when_set() {
	assert_eq!(
		substitute("${X-fallback}", &vars(&[("X", "v")])).unwrap(),
		"v"
	);
}

// ${VAR:+alt}

#[test]
fn alt_if_set_and_nonempty_when_unset() {
	assert_eq!(substitute("${X:+alt}", &vars(&[])).unwrap(), "");
}

#[test]
fn alt_if_set_and_nonempty_when_empty() {
	assert_eq!(substitute("${X:+alt}", &vars(&[("X", "")])).unwrap(), "");
}

#[test]
fn alt_if_set_and_nonempty_when_set() {
	assert_eq!(
		substitute("${X:+alt}", &vars(&[("X", "v")])).unwrap(),
		"alt"
	);
}

// ${VAR+alt}

#[test]
fn alt_if_set_when_unset() {
	assert_eq!(substitute("${X+alt}", &vars(&[])).unwrap(), "");
}

#[test]
fn alt_if_set_when_empty_returns_alt() {
	assert_eq!(substitute("${X+alt}", &vars(&[("X", "")])).unwrap(), "alt");
}

// ${VAR:?msg}

#[test]
fn error_if_unset_or_empty_when_unset() {
	assert!(substitute("${X:?required}", &vars(&[])).is_err());
}

#[test]
fn error_if_unset_or_empty_when_empty() {
	assert!(substitute("${X:?required}", &vars(&[("X", "")])).is_err());
}

#[test]
fn error_if_unset_or_empty_when_set() {
	assert_eq!(
		substitute("${X:?required}", &vars(&[("X", "ok")])).unwrap(),
		"ok"
	);
}

// ${VAR?msg}

#[test]
fn error_if_unset_when_unset() {
	assert!(substitute("${X?required}", &vars(&[])).is_err());
}

#[test]
fn error_if_unset_when_empty_returns_empty() {
	assert_eq!(
		substitute("${X?required}", &vars(&[("X", "")])).unwrap(),
		""
	);
}

// Nested interpolation inside modifier values (compose-spec allows nesting).

#[test]
fn nested_default_is_interpolated() {
	// ${FOO:-${BAR}} → BAR's value when FOO is unset.
	assert_eq!(
		substitute("${FOO:-${BAR}}", &vars(&[("BAR", "b")])).unwrap(),
		"b"
	);
}

#[test]
fn nested_chained_default_falls_through() {
	// ${FOO:-${BAR:-baz}} → literal baz when both FOO and BAR are unset.
	assert_eq!(
		substitute("${FOO:-${BAR:-baz}}", &vars(&[])).unwrap(),
		"baz"
	);
}

#[test]
fn nested_alt_is_interpolated() {
	// ${FOO:+${BAR}} → BAR's value when FOO is set and non-empty.
	assert_eq!(
		substitute("${FOO:+${BAR}}", &vars(&[("FOO", "x"), ("BAR", "b")])).unwrap(),
		"b"
	);
}

#[test]
fn nested_default_with_trailing_text() {
	// The balanced-brace scan stops at the matching close, leaving following text.
	assert_eq!(
		substitute("${FOO:-${BAR}}/tail", &vars(&[("BAR", "b")])).unwrap(),
		"b/tail"
	);
}

/// A `{` inside a default is text, as in Docker Compose: only `${` opens a
/// nested reference. `${A:-{}` used to fail as unterminated.
#[test]
fn a_brace_inside_a_default_is_text() {
	for (template, unset, set) in [
		("${A:-{}", "{", "a"),
		("${A:-{x}", "{x", "a"),
		("${A:-a{b}c}", "a{b}c", "a"),
		("${A:-${B:-{}}", "{", "a"),
	] {
		assert_eq!(
			substitute(template, &vars(&[])).unwrap(),
			unset,
			"{template}"
		);
		assert_eq!(
			substitute(template, &vars(&[("A", "a")])).unwrap(),
			set,
			"{template}"
		);
	}
}

/// The message of `:?` is interpolated with the same variables, as Docker does.
#[test]
fn the_message_of_a_required_variable_is_interpolated() {
	let err = substitute("${MISSING:?${B}}", &vars(&[("B", "bval")])).unwrap_err();
	assert!(err.to_string().contains("bval"), "{err}");
	assert!(!err.to_string().contains("${B}"), "{err}");
}

/// The forms Docker refuses as `invalid interpolation format` are refused; the
/// valid ones next to them still resolve.
#[test]
fn an_invalid_interpolation_format_is_refused() {
	for template in ["${A:foo}", "${A:=foo}", "${A", "x ${A:=b} y"] {
		assert!(
			substitute(template, &vars(&[("A", "set")])).is_err(),
			"{template}"
		);
	}
	for (template, want) in [
		("$$", "$"),
		("a$", "a$"),
		("$A", "set"),
		("${A}", "set"),
		("${A-d}", "set"),
		("${A:+alt}", "alt"),
		("${A+alt}", "alt"),
		("${A:?m}", "set"),
		("${A?m}", "set"),
	] {
		assert_eq!(
			substitute(template, &vars(&[("A", "set")])).unwrap(),
			want,
			"{template}"
		);
	}
}

/// The refused template comes from the compose file; a terminal escape in it
/// is shown escaped and a long one is cut, so the error cannot drive the
/// terminal or flood it.
#[test]
fn a_refused_template_is_escaped_and_bounded_in_the_message() {
	let hostile = format!("${{A:\u{1b}[31m{}}}", "x".repeat(10_000));
	let err = substitute(&hostile, &vars(&[("A", "set")]))
		.unwrap_err()
		.to_string();
	assert!(!err.contains('\u{1b}'), "raw escape in: {err:?}");
	assert!(err.len() < 1_000, "{} bytes", err.len());
}
