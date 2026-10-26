use super::*;

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
