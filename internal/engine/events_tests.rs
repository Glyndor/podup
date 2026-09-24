use super::{build_event_filters, format_event, validate_events_since, TIME_WIDTH};
use serde_json::json;

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

#[test]
fn formats_docker_compat_shape() {
	let ev = json!({
		"Type": "container",
		"Action": "start",
		"Actor": { "Attributes": { "name": "web-1" } },
		"time": 0,
	});
	// Columns are fixed-width now (#1248): TIME leads, `container` fills TYPE
	// exactly, `start` is padded out to ACTION, and NAME is the trailing raw
	// column.
	//
	// The TIME cell is asserted by width rather than by value: it renders
	// the reader's wall clock, so a fixed string would pass on a machine at
	// -05:00 and fail on a runner at UTC. `crate::timestamp` pins what goes
	// in the cell; this pins that the columns after it land where the header
	// says.
	let out = format_event(&ev, false);
	assert_eq!(
		&out[TIME_WIDTH..],
		" container start          web-1",
		"columns after TIME drifted: {out:?}"
	);
}

#[test]
fn formats_libpod_native_shape() {
	let ev = json!({ "Type": "container", "status": "die", "id": "abc123", "time": 0 });
	let out = format_event(&ev, false);
	assert_eq!(
		&out[TIME_WIDTH..],
		" container die            abc123",
		"columns after TIME drifted: {out:?}"
	);
}

#[test]
fn json_mode_emits_raw_object() {
	let ev = json!({ "Type": "container", "Action": "start" });
	let out = format_event(&ev, true);
	assert!(out.contains("\"Type\":\"container\""));
	assert!(out.contains("\"Action\":\"start\""));
}

/// #1896: libpod reads a relative `since` as a time before now, so `-30m` is
/// thirty minutes in the future and a window starting there matches nothing.
/// The validator has to reject it before any request hits libpod.
#[test]
fn events_since_rejects_a_negative_relative_time() {
	let err = validate_events_since(Some("-30m"))
		.expect_err("--since -30m must be rejected, not silently sent to libpod");
	let msg = format!("{err}");
	assert!(
		msg.contains("--since 30m"),
		"the error must suggest the value without the leading '-'; got {msg}"
	);

	validate_events_since(Some("-1h30m")).expect_err("--since -1h30m must be rejected");

	for ok in [
		None,
		Some("30m"),
		Some("1h30m"),
		Some("1700000000"),
		Some("2026-01-01T00:00:00Z"),
		Some("2026-01-01T00:00:00-05:00"),
	] {
		validate_events_since(ok)
			.unwrap_or_else(|e| panic!("{ok:?} should be accepted but got {e}"));
	}
}
