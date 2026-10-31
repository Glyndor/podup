use super::super::filters::status_matches;
use super::*;

#[test]
fn status_matches_empty_filter_matches_all() {
	assert!(status_matches("running", &[]));
	assert!(status_matches("exited", &[]));
}

#[test]
fn status_matches_is_case_insensitive_prefix() {
	assert!(status_matches("running", &["RUNNING".to_string()]));
	assert!(status_matches("exited", &["exit".to_string()]));
	assert!(!status_matches("running", &["exited".to_string()]));
	// An empty wanted value never matches.
	assert!(!status_matches("running", &["".to_string()]));
}

#[test]
fn split_ps_filters_buckets_known_keys_and_flags_unknown() {
	let (status, names, unknown) = split_ps_filters(&[
		"status=running".to_string(),
		"name=web".to_string(),
		"label=foo".to_string(),
	]);
	assert_eq!(status, vec!["running".to_string()]);
	assert_eq!(names, vec!["web".to_string()]);
	assert_eq!(unknown, vec!["label=foo".to_string()]);
}

#[test]
fn display_status_falls_back_to_state_when_status_empty() {
	assert_eq!(display_status(&entry("", "running")), "running");
	assert_eq!(display_status(&entry("", "exited")), "exited");
}

#[test]
fn display_status_prefers_status_when_present() {
	assert_eq!(
		display_status(&entry("Up 2 seconds", "running")),
		"Up 2 seconds"
	);
}
