use super::{
	build_event_filters, format_event, rename_event, validate_events_since, Engine, EventsOptions,
	TIME_WIDTH,
};
use crate::libpod::Client;
use serde_json::{json, Value};

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

/// The libpod `/events` endpoint names a container's death `died` (not
/// `die`); podup's user-facing output keeps the docker-compat verb so a
/// `--filter event=die` still matches a container death on libpod
/// (#1914). Other verbs pass through under the `Action` key. The
/// full compat rewrite rules (image+remove -> delete, died -> die +
/// exitCode copy, container remove unchanged) are pinned in
/// `events_compat_rewrite_tests`.
#[test]
fn rename_event_promotes_status_to_action_and_rewrites_died() {
	for (raw, want_action, want_status) in [
		("died", "die", "die"),
		("start", "start", "start"),
		("die", "die", "die"),
		("delete", "delete", "delete"),
	] {
		let v = json!({ "Type": "container", "status": raw });
		let out = rename_event(&v);
		assert_eq!(
			out.get("Action").and_then(|v| v.as_str()),
			Some(want_action),
			"verb {raw:?} did not map to Action={want_action:?}: {out:?}"
		);
		assert_eq!(
			out.get("status").and_then(|v| v.as_str()),
			Some(want_status),
			"verb {raw:?} did not map to status={want_status:?}: {out:?}"
		);
	}
}

/// `Actor.Attributes.containerExitCode` (libpod) is copied into the
/// docker-compat `Actor.Attributes.exitCode` so a script that reads the
/// docker-compat key still finds the value, while a script that reads
/// the libpod key still finds its value (#1914). The compat build
/// handler did the same: `exitCode` was set from `containerExitCode`
/// while `containerExitCode` stayed put. Removing the libpod key would
/// silently break every caller keyed on it.
#[test]
fn rename_event_copies_container_exit_code_into_exit_code() {
	let v = json!({
		"Type": "container",
		"status": "died",
		"Actor": {
			"Attributes": {
				"name": "web-1",
				"containerExitCode": "3",
			}
		}
	});
	let out = rename_event(&v);
	assert_eq!(
		out.pointer("/Actor/Attributes/exitCode")
			.and_then(|v| v.as_str()),
		Some("3"),
		"containerExitCode was not copied into exitCode: {out:?}"
	);
	assert_eq!(
		out.pointer("/Actor/Attributes/containerExitCode")
			.and_then(|v| v.as_str()),
		Some("3"),
		"containerExitCode must remain alongside the docker-compat exitCode: {out:?}"
	);
	assert_eq!(
		out.pointer("/Actor/Attributes/name")
			.and_then(|v| v.as_str()),
		Some("web-1"),
		"name must remain: {out:?}"
	);
}

/// The compat handler set both `status` and `Action` when it rewrote a
/// verb (`status` is the libpod-native verb key, `Action` is the
/// docker-compat one; both land at `die` for a container death).
/// Verbs that are not rewritten pass through unchanged under both
/// keys (#1914).
#[test]
fn rename_event_keeps_status_alongside_action() {
	let v = json!({
		"Type": "container",
		"status": "died",
		"Actor": { "Attributes": { "name": "web-1" } },
	});
	let out = rename_event(&v);
	assert_eq!(
		out.get("Action").and_then(Value::as_str),
		Some("die"),
		"status=died must become Action=die: {out:?}"
	);
	assert_eq!(
		out.get("status").and_then(Value::as_str),
		Some("die"),
		"status=died must become status=die (the compat handler set both keys): {out:?}"
	);
}

/// The table form must render `died` as `die` and `remove` as `delete`,
/// the docker-compat verbs podup has always printed (#1914).
#[test]
fn table_form_renders_libpod_verbs_as_docker_compat() {
	let died = json!({ "Type": "container", "status": "died", "id": "web-1", "time": 0 });
	let out = format_event(&died, false);
	assert!(
		out.contains("die"),
		"libpod `died` must render as docker-compat `die`: {out:?}"
	);
	assert!(
		!out.contains("died"),
		"libpod `died` must not appear in the table form: {out:?}"
	);

	let remove = json!({ "Type": "image", "status": "remove", "id": "img-1", "time": 0 });
	let out = format_event(&remove, false);
	assert!(
		out.contains("delete"),
		"libpod `remove` must render as docker-compat `delete`: {out:?}"
	);
}

#[test]
fn json_mode_emits_raw_object() {
	let ev = json!({ "Type": "container", "Action": "start" });
	let out = format_event(&ev, true);
	assert!(out.contains("\"Type\":\"container\""));
	assert!(out.contains("\"Action\":\"start\""));
}

/// The JSON mode routes through `rename_event`, which copies
/// `containerExitCode` into `exitCode` while leaving the libpod key in
/// place. A `--format json` consumer keyed on `containerExitCode`
/// (libpod scripts predating #1914) and one keyed on `exitCode`
/// (docker-compat scripts) both find the same value (#1914).
#[test]
fn json_mode_emits_both_exit_code_keys() {
	let ev = json!({
		"Type": "container",
		"status": "died",
		"Actor": {
			"Attributes": {
				"name": "web-1",
				"containerExitCode": "137",
			}
		}
	});
	let out = format_event(&ev, true);
	let parsed: serde_json::Value =
		serde_json::from_str(out.trim()).expect("json mode emits one JSON object per line");
	assert_eq!(
		parsed
			.pointer("/Actor/Attributes/containerExitCode")
			.and_then(|v| v.as_str()),
		Some("137"),
		"the libpod containerExitCode key must remain on the JSON wire: {parsed:?}"
	);
	assert_eq!(
		parsed
			.pointer("/Actor/Attributes/exitCode")
			.and_then(|v| v.as_str()),
		Some("137"),
		"the docker-compat exitCode key must be set on the JSON wire: {parsed:?}"
	);
	assert_eq!(
		parsed.get("Action").and_then(Value::as_str),
		Some("die"),
		"libpod `died` must promote to docker-compat `die` on the JSON wire: {parsed:?}"
	);
	assert_eq!(
		parsed.get("status").and_then(Value::as_str),
		Some("die"),
		"the compat handler rewrote `status` to `die` too: {parsed:?}"
	);
}

/// #1896: libpod reads a relative `since` as a time before now, so `-30m` is
/// thirty minutes in the future and a window starting there matches nothing.
/// The validator has to reject it before any request hits libpod.
///
/// The earlier check also rejected `-1`/`-1.5` (valid pre-epoch Unix
/// timestamps) and `-0s` (a zero offset, i.e. "now"), while letting `-.5h`
/// through; Go parses that as `-30m`, so it reproduced the bug. The rule
/// is now: reject only when the part after `-` parses exactly as a Go
/// duration (one or more `<number><unit>` segments, with at least one
/// non-zero digit overall). A string like `-1e3` is not a Go duration, so
/// it is forwarded to libpod, which reads it as a negative Unix timestamp.
#[test]
fn events_since_rejects_a_negative_relative_time() {
	for bad in ["-30m", "-1h30m", "-30s", "-.5h", "-1.5h", "-10ms", "-2us"] {
		validate_events_since(Some(bad)).expect_err(&format!(
			"{bad:?} must be rejected, not silently sent to libpod"
		));
	}
	let err = validate_events_since(Some("-30m"))
		.expect_err("--since -30m must be rejected, not silently sent to libpod");
	let msg = format!("{err}");
	assert!(
		msg.contains("--since 30m"),
		"the error must suggest the value without the leading '-'; got {msg}"
	);

	for ok in [
		None,
		Some("30m"),
		Some("1h30m"),
		Some("-1"),
		Some("-1.5"),
		Some("-0s"),
		Some("-0m"),
		Some("-1e3"),
		Some("-1E3"),
		Some("-1.5e2"),
		Some("1700000000"),
		Some("2026-01-01T00:00:00Z"),
		Some("2026-01-01T00:00:00-05:00"),
	] {
		validate_events_since(ok)
			.unwrap_or_else(|e| panic!("{ok:?} should be accepted but got {e}"));
	}
}

/// #1896: the production call site has to reject a negative relative
/// `--since` before any request goes out. The previous private-validator
/// test left the call itself uncovered: removing
/// `validate_events_since(opts.since.as_deref())?` at the top of
/// `stream_events_with_options` reverted the bug without invalidating the
/// other assertions. Pointing the engine at a path that does not exist
/// proves validation runs first: a connection error would mean the request
/// was already in flight.
#[tokio::test]
async fn stream_events_rejects_a_negative_since_before_any_request() {
	let dir = tempfile::tempdir().expect("tempdir");
	let sock = dir.path().join("missing.sock");
	let engine = Engine::with_base_dir(
		Client::new(sock.to_string_lossy().into_owned()),
		"proj".into(),
		dir.path().to_path_buf(),
	);

	let opts = EventsOptions::new(Some("-30m".into()), None, vec![]);
	let err = engine
		.stream_events_with_options(false, &opts)
		.await
		.expect_err("--since -30m must be rejected before the events stream is opened");
	let msg = format!("{err}");
	assert!(
		msg.contains("--since 30m"),
		"the error must suggest the value without the leading '-'; got {msg}"
	);
}
