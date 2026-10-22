use super::*;

#[test]
fn build_event_filters_scopes_to_project_label() {
	let f = build_event_filters("demo", &[]).unwrap();
	assert_eq!(f, json!({ "label": ["podup.project=demo"] }));
}

#[test]
fn build_event_filters_merges_user_predicates() {
	let f = build_event_filters(
		"demo",
		&[
			"event=start".to_string(),
			"event=die".to_string(),
			"type=container".to_string(),
		],
	)
	.unwrap();
	assert_eq!(
		f,
		json!({
			"label": ["podup.project=demo"],
			"event": ["start", "die"],
			"type": ["container"],
		})
	);
}

/// #1081: a predicate with no `=` used to be dropped, so `events --filter
/// garbage` silently scoped to the whole project and printed everything, a
/// caller reads that back as "these all matched".
#[test]
fn malformed_filter_is_rejected_not_dropped() {
	let err = build_event_filters("demo", &["bogus".to_string()])
		.expect_err("a filter with no `=` must not be silently ignored");
	assert!(format!("{err}").contains("bogus"), "got {err}");
}
