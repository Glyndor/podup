use super::{is_valid_log_time, validate_log_filters};
use crate::engine::query::LogsOptions;

#[test]
fn validate_log_filters_accepts_good_values() {
	assert!(validate_log_filters(&LogsOptions {
		tail: Some("all".into()),
		since: Some("10m".into()),
		until: Some("2024-01-01T00:00:00Z".into()),
		..Default::default()
	})
	.is_ok());
	assert!(validate_log_filters(&LogsOptions {
		tail: Some("100".into()),
		since: Some("1700000000".into()),
		..Default::default()
	})
	.is_ok());
	assert!(validate_log_filters(&LogsOptions::default()).is_ok());
}

#[test]
fn validate_log_filters_rejects_bad_tail_and_time() {
	assert!(validate_log_filters(&LogsOptions {
		tail: Some("abc".into()),
		..Default::default()
	})
	.is_err());
	assert!(validate_log_filters(&LogsOptions {
		since: Some("yesterday".into()),
		..Default::default()
	})
	.is_err());
	assert!(validate_log_filters(&LogsOptions {
		until: Some("not-a-time".into()),
		..Default::default()
	})
	.is_err());
}

#[test]
fn is_valid_log_time_classifies_forms() {
	assert!(is_valid_log_time("10m"));
	assert!(is_valid_log_time("1h30m"));
	assert!(is_valid_log_time("500ms"));
	assert!(is_valid_log_time("1700000000"));
	assert!(is_valid_log_time("2024-01-02T03:04:05Z"));
	assert!(!is_valid_log_time("abc"));
	assert!(!is_valid_log_time(""));
	assert!(!is_valid_log_time("10x"));
}
