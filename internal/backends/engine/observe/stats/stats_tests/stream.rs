use super::*;

// --- the stream that breaks mid sample -----------------------------------

fn name_set(names: &[&str]) -> HashSet<String> {
	names.iter().map(|s| s.to_string()).collect()
}

#[test]
fn stats_stream_break_is_fatal_only_while_a_sampled_container_still_runs() {
	let sampled = name_set(&["proj-web-1", "proj-db-1"]);

	// A sampled container is still running: the stream truncated a live sample,
	// so the error is a real failure (#1080), the exit code a monitor needs.
	assert!(stats_stream_broke_mid_sample(
		&sampled,
		&name_set(&["proj-web-1"])
	));

	// Every sampled container has stopped: the stream ended because there was
	// nothing left to sample, and the missing terminal frame is the
	// finished-vs-broken ambiguity (#1104), not a fault.
	assert!(!stats_stream_broke_mid_sample(&sampled, &HashSet::new()));

	// A different, unrelated container is running (e.g. another project): it was
	// never sampled, so it does not make this stream's end a failure.
	assert!(!stats_stream_broke_mid_sample(
		&sampled,
		&name_set(&["other-app-1"])
	));
}

#[test]
fn stats_stream_break_with_no_sampled_containers_is_never_fatal() {
	// Nothing was being sampled, so no end can be a truncated-sample failure.
	assert!(!stats_stream_broke_mid_sample(
		&HashSet::new(),
		&name_set(&["proj-web-1"])
	));
}
