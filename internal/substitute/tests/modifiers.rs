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
