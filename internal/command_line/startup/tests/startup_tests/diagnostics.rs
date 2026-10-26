use super::*;

#[test]
fn level_words_match_user_facing_terms() {
	assert_eq!(level_word(tracing::Level::WARN), "warning");
	assert_eq!(level_word(tracing::Level::ERROR), "error");
	assert_eq!(level_word(tracing::Level::INFO), "info");
	assert_eq!(level_word(tracing::Level::DEBUG), "debug");
	assert_eq!(level_word(tracing::Level::TRACE), "trace");
	// Each severity gets a distinct style; debug/trace share the dim style.
	assert_ne!(
		level_style(tracing::Level::ERROR),
		level_style(tracing::Level::INFO)
	);
	assert_ne!(
		level_style(tracing::Level::WARN),
		level_style(tracing::Level::ERROR)
	);
	assert_eq!(
		level_style(tracing::Level::DEBUG),
		level_style(tracing::Level::TRACE)
	);
}

#[test]
fn broken_pipe_panic_detected() {
	assert!(is_broken_pipe_panic(
		"failed printing to stdout: Broken pipe (os error 32)"
	));
	assert!(is_broken_pipe_panic(
		"failed printing to stderr: Broken pipe (os error 32)"
	));
	assert!(!is_broken_pipe_panic("some other internal error"));
}

#[test]
fn an_unrelated_panic_mentioning_a_broken_pipe_is_not_swallowed() {
	// These are real crashes. Matching them would exit 0 and print nothing,
	// and `panic = "abort"` leaves this hook as the only gate before the exit
	// status, so the process would report success on a genuine bug.
	assert!(!is_broken_pipe_panic("Broken pipe"));
	assert!(!is_broken_pipe_panic(
		"called `Result::unwrap()` on an `Err` value: Os { code: 32, kind: BrokenPipe, message: \"Broken pipe\" }"
	));
	assert!(!is_broken_pipe_panic(
		"podman refused the request: broken pipe reading from the container"
	));
	assert!(!is_broken_pipe_panic("assertion failed at os error 32"));
}

#[test]
fn internal_error_notice_reports_and_warns_on_secrets() {
	let notice = internal_error_notice();
	assert!(notice.contains(REPO_URL), "points at the issue tracker");
	assert!(notice.contains("/issues"));
	assert!(
		notice.contains("redact"),
		"reminds the user to scrub secrets"
	);
	assert!(
		notice.contains("RUST_LOG=debug"),
		"tells the user what to capture"
	);
}
