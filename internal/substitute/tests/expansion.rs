use super::*;

// Plain passthrough

#[test]
fn plain_text_unchanged() {
	assert_eq!(
		substitute("hello world", &vars(&[])).unwrap(),
		"hello world"
	);
}

#[test]
fn dollar_at_end_emitted_literally() {
	assert_eq!(substitute("price$", &vars(&[])).unwrap(), "price$");
}

#[test]
fn double_dollar_becomes_single() {
	assert_eq!(substitute("$$", &vars(&[])).unwrap(), "$");
}

// $VAR (unbraced)

#[test]
fn unbraced_var_set_expands() {
	assert_eq!(substitute("$FOO", &vars(&[("FOO", "bar")])).unwrap(), "bar");
}

#[test]
fn unbraced_var_unset_expands_to_empty() {
	assert_eq!(substitute("$MISSING", &vars(&[])).unwrap(), "");
}

#[test]
fn unbraced_var_followed_by_non_ident() {
	assert_eq!(substitute("$FOO!", &vars(&[("FOO", "x")])).unwrap(), "x!");
}

// ${VAR} (braced, no modifier)

#[test]
fn braced_var_set_expands() {
	assert_eq!(
		substitute("${FOO}", &vars(&[("FOO", "val")])).unwrap(),
		"val"
	);
}

#[test]
fn braced_var_unset_expands_to_empty() {
	assert_eq!(substitute("${MISSING}", &vars(&[])).unwrap(), "");
}

// Malformed / pathological references

#[test]
fn empty_name_is_error() {
	assert!(substitute("${}", &vars(&[])).is_err());
}

#[test]
fn digit_leading_name_is_error() {
	assert!(substitute("${1BAD}", &vars(&[])).is_err());
}

#[test]
fn unterminated_modifier_is_error() {
	// The missing `}` must error rather than consume the rest of the input.
	assert!(substitute("${TAG:-latest\nmore", &vars(&[])).is_err());
}

// Multiple substitutions in one string

#[test]
fn multiple_vars_in_string() {
	let v = vars(&[("A", "hello"), ("B", "world")]);
	assert_eq!(substitute("$A ${B}!", &v).unwrap(), "hello world!");
}
