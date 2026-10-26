use super::*;

#[test]
fn validate_project_name_message_matches_the_enforced_rule() {
	// Pins the error text to what `is_safe_project_name` actually enforces
	// (lowercase-only, no '.'), not the looser rule the message used to
	// describe. `My.App` is exactly the kind of name the old wording
	// ("ASCII letters, digits, '-', '_', '.'") implied was fine, yet
	// `is_safe_project_name` has always rejected it (uppercase and '.'
	// are both disallowed) - a user following the old message would still
	// get bounced.
	let err = validate_project_name("My.App").unwrap_err();
	let msg = err.to_string();
	assert!(
		msg.contains("lowercase"),
		"message must say the name must be lowercase: {msg:?}"
	);
	assert!(
		!msg.contains("'.'"),
		"message must not list '.' as an allowed character: {msg:?}"
	);
}

#[test]
fn validate_project_name_accepts_a_safe_name() {
	assert!(validate_project_name("my-app").is_ok());
}
