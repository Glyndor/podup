use super::{log_query, stream_broke_mid_output};
use crate::engine::query::LogsOptions;

#[test]
fn log_query_defaults_to_stdout_stderr_no_follow() {
	let q = log_query(&LogsOptions::default());
	assert_eq!(q, "stdout=true&stderr=true&follow=false&timestamps=false");
}

#[test]
fn log_query_includes_set_options() {
	let q = log_query(&LogsOptions {
		follow: true,
		tail: Some("20".into()),
		since: Some("10m".into()),
		until: Some("2024-01-01T00:00:00".into()),
		timestamps: true,
	});
	assert!(q.contains("follow=true"));
	assert!(q.contains("timestamps=true"));
	assert!(q.contains("&tail=20"));
	assert!(q.contains("&since=10m"));
	// `:` is percent-encoded in the query value.
	assert!(q.contains("&until=2024-01-01T00%3A00%3A00"));
}

// ---------------------------------------------------------------------------
// #1104: telling a finished log stream from a broken one
// ---------------------------------------------------------------------------

#[test]
fn a_stream_ending_while_the_container_runs_is_a_break() {
	// The container outlived its own log stream, so output was truncated.
	assert!(stream_broke_mid_output(Some(true)));
}

#[test]
fn a_stream_ending_after_the_container_stopped_is_clean() {
	// The stream is supposed to end here; libpod just did not get to say so.
	assert!(!stream_broke_mid_output(Some(false)));
}

#[test]
fn an_unreadable_state_counts_as_a_break() {
	// The rule that is easy to get backwards, and the one that matters most: the
	// severed connection that ends the stream is usually the same one the
	// re-check needs, so reading "could not tell" as a clean end turns every
	// transport failure back into exit 0. Measured: the permissive version
	// reported a still-running container as stopped when the socket restarted
	// under an attached `logs -f`.
	assert!(stream_broke_mid_output(None));
}
