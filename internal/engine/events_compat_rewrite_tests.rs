use super::{format_event, rename_event};
use serde_json::{json, Value};

/// Mirror the docker-compat events handler exactly: `Action`/`status` are
/// only rewritten for the two cases the handler rewrote.
///
/// Fixture is built from the libpod shape (verb in `status`, no `Action`),
/// since that is what podup reads from `libpod/events`. A `container`
/// death is the case the compat handler rewrites to `die` while copying
/// `containerExitCode` into `exitCode`; both keys carry the same value
/// afterwards so a script keyed on either still finds it (#1914).
#[test]
fn died_container_action_and_status_become_die_with_exit_code_keys_equal() {
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
		out.get("Action").and_then(Value::as_str),
		Some("die"),
		"libpod `died` must promote to docker-compat `die`: {out:?}"
	);
	assert_eq!(
		out.get("status").and_then(Value::as_str),
		Some("die"),
		"the compat handler sets `status` too; libpod `died` must become `die`: {out:?}"
	);
	assert_eq!(
		out.pointer("/Actor/Attributes/exitCode")
			.and_then(Value::as_str),
		Some("3"),
		"the docker-compat `exitCode` key must hold the containerExitCode value: {out:?}"
	);
	assert_eq!(
		out.pointer("/Actor/Attributes/containerExitCode")
			.and_then(Value::as_str),
		Some("3"),
		"the libpod `containerExitCode` key must stay alongside the docker-compat key: {out:?}"
	);
}

/// `exec_died` is a distinct verb (it signals an `exec` session ending,
/// not the container itself dying). The compat handler only rewrote
/// `Action == "died"`, so `exec_died` must pass through unchanged and
/// must NOT pick up an `exitCode` copy: the previous code copied
/// `containerExitCode` into `exitCode` whenever the attribute existed,
/// regardless of verb, and `podman exec` events carry the libpod key
/// even though no docker-compat `exitCode` was ever published for them
/// (#1914).
#[test]
fn exec_died_event_passes_through_with_no_exit_code_copy() {
	let v = json!({
		"Type": "container",
		"status": "exec_died",
		"Actor": {
			"Attributes": {
				"name": "web-1",
				"containerExitCode": "137",
			}
		}
	});
	let out = rename_event(&v);
	assert_eq!(
		out.get("Action").and_then(Value::as_str),
		Some("exec_died"),
		"exec_died must not be collapsed into `die`: {out:?}"
	);
	assert_eq!(
		out.get("status").and_then(Value::as_str),
		Some("exec_died"),
		"the libpod status key must stay at exec_died: {out:?}"
	);
	assert!(
		out.pointer("/Actor/Attributes/exitCode").is_none(),
		"exec_died must not gain an exitCode copy: {out:?}"
	);
}

/// A container removal is `Action=remove` in libpod, and that is what
/// podup has always emitted on the docker-compat path. The compat
/// handler only rewrote `remove` -> `delete` when `Type == "image"`,
/// so a container `remove` must pass through untouched. The previous
/// code rewrote it for every `Type`, which turned a container removal
/// into a `delete` and silently changed the verb a `--filter event=...`
/// call had to match (#1914).
#[test]
fn container_remove_event_passes_through_unchanged() {
	let v = json!({
		"Type": "container",
		"status": "remove",
		"Actor": { "Attributes": { "name": "web-1" } }
	});
	let out = rename_event(&v);
	assert_eq!(
		out.get("Action").and_then(Value::as_str),
		Some("remove"),
		"container remove must stay `remove`; compat only rewrites it for `Type == image`: {out:?}"
	);
	assert_eq!(
		out.get("status").and_then(Value::as_str),
		Some("remove"),
		"the libpod status key must stay at remove for a container: {out:?}"
	);
}

/// An image removal is `Action=remove` in libpod and `Action=delete` in
/// docker-compat: that is the rewrite the compat handler applied, on
/// `Type == "image"` AND `Action == "remove"` only. Both `Action` and
/// `status` are set to `delete`, mirroring the handler's two writes
/// (#1914).
#[test]
fn image_remove_event_action_and_status_become_delete() {
	let v = json!({
		"Type": "image",
		"status": "remove",
		"Actor": { "Attributes": { "name": "img-1" } }
	});
	let out = rename_event(&v);
	assert_eq!(
		out.get("Action").and_then(Value::as_str),
		Some("delete"),
		"image remove must become `delete`: {out:?}"
	);
	assert_eq!(
		out.get("status").and_then(Value::as_str),
		Some("delete"),
		"the compat handler sets `status` to `delete` too: {out:?}"
	);
}

/// The table path reads the same two rules. A container `remove`
/// stays `remove` on the line a reader scans, and an image `remove`
/// renders as `delete` (the docker-compat verb the table has always
/// shown). The rule is the same one the JSON path applies (#1914).
#[test]
fn table_path_mirrors_compat_image_only_remove_rewrite() {
	let container = json!({
		"Type": "container",
		"status": "remove",
		"id": "web-1",
		"time": 0,
	});
	let out = format_event(&container, false);
	assert!(
		out.contains(" remove "),
		"container `remove` must stay `remove` on the table path: {out:?}"
	);
	assert!(
		!out.contains(" delete "),
		"container `remove` must not be rewritten as `delete` on the table path: {out:?}"
	);

	let image = json!({
		"Type": "image",
		"status": "remove",
		"id": "img-1",
		"time": 0,
	});
	let out = format_event(&image, false);
	assert!(
		out.contains(" delete "),
		"image `remove` must render as `delete` on the table path: {out:?}"
	);
}
