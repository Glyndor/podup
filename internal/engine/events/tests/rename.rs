use super::*;

/// The libpod `/events` endpoint names a container's death `died` (not
/// `die`); podup's user-facing output keeps the docker-compat verb so a
/// `--filter event=die` still matches a container death on libpod
/// (#1914). Other verbs pass through under the `Action` key. The
/// full compat rewrite rules (image+remove -> delete, died -> die +
/// exitCode copy, container remove unchanged) are pinned in
/// `compat_rewrite_tests`.
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
