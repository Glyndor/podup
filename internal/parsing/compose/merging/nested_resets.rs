//! `!reset` on a part inside a service key: one entry of `depends_on`, `labels`
//! or `networks`, `healthcheck.interval`, `deploy.resources`.
//!
//! The tags of [`super::tags`] act on a whole key of a service. A reset one
//! level down names a single piece of what an earlier file defined, and the
//! typed model cannot carry it: a reset entry is a null or an empty mapping,
//! which the entry's type either refuses (`depends_on`, `labels`) or reads as a
//! value of its own (`interval`). So the path is read from the raw document,
//! the entry is dropped from it before typing, and the path is removed from the
//! merged service afterwards.

use std::collections::HashMap;

use serde_yaml::Value;

use crate::compose::types::Service;

/// Key paths below a service key that a document resets, per service name.
/// Each path starts with the service key, as in `["depends_on", "db"]`.
pub(crate) type NestedResets = HashMap<String, Vec<Vec<String>>>;

fn walk(path: &mut Vec<String>, value: &Value, out: &mut Vec<Vec<String>>) {
	match value {
		Value::Tagged(t) if t.tag != "!reset" => walk(path, &t.value, out),
		Value::Mapping(map) => {
			for (key, child) in map {
				let Some(key) = key.as_str() else { continue };
				path.push(key.to_string());
				if matches!(child, Value::Tagged(t) if t.tag == "!reset") {
					out.push(path.clone());
				} else {
					walk(path, child, out);
				}
				path.pop();
			}
		}
		_ => {}
	}
}

/// The paths below a service key that `raw` resets. An entry of a list cannot
/// be named, and a job is refused elsewhere, so only mappings of services count.
pub(crate) fn nested_resets(raw: &Value) -> NestedResets {
	let mut found = NestedResets::new();
	let Some(services) = raw.get("services").and_then(Value::as_mapping) else {
		return found;
	};
	for (name, service) in services {
		let (Some(name), Some(fields)) = (name.as_str(), service.as_mapping()) else {
			continue;
		};
		let mut paths = Vec::new();
		for (key, value) in fields {
			let Some(key) = key.as_str() else { continue };
			// A tag on the key itself is the whole-key case.
			if !matches!(value, Value::Tagged(t) if t.tag == "!reset" || t.tag == "!override") {
				walk(&mut vec![key.to_string()], value, &mut paths);
			}
		}
		if !paths.is_empty() {
			found.insert(name.to_string(), paths);
		}
	}
	found
}

/// [`nested_resets`] of a compose file on disk; empty when it cannot be read.
pub(crate) fn nested_resets_from_file(path: &std::path::Path) -> NestedResets {
	let Ok(text) = std::fs::read_to_string(path) else {
		return NestedResets::new();
	};
	serde_yaml::from_str::<Value>(&text)
		.map(|raw| nested_resets(&raw))
		.unwrap_or_default()
}

fn remove_path(root: &mut Value, path: &[String]) {
	let Some((last, parents)) = path.split_last() else {
		return;
	};
	let mut node = root;
	for key in parents {
		node = match node.get_mut(key.as_str()) {
			Some(Value::Tagged(t)) => &mut t.value,
			Some(next) => next,
			None => return,
		};
	}
	if let Some(map) = node.as_mapping_mut() {
		map.remove(last.as_str());
	}
}

/// Remove the reset paths from the document so they are never typed.
pub(crate) fn drop_nested_resets(raw: &mut Value) {
	let Some(services) = raw.get_mut("services") else {
		return;
	};
	for (name, paths) in nested_resets(&Value::Mapping(
		[(Value::from("services"), services.clone())]
			.into_iter()
			.collect(),
	)) {
		for path in paths {
			if let Some(service) = services.get_mut(name.as_str()) {
				remove_path(service, &path);
			}
		}
	}
}

/// Remove the reset paths from a merged service. The service goes through its
/// own serialized form, which carries every key a file can set; what is only
/// bookkeeping for the merge is put back after.
pub(crate) fn apply_nested_resets(service: &mut Service, paths: &[Vec<String>]) {
	let Ok(mut value) = serde_yaml::to_value(&*service) else {
		return;
	};
	for path in paths {
		remove_path(&mut value, path);
	}
	let Ok(mut rebuilt) = serde_yaml::from_value::<Service>(value) else {
		return;
	};
	rebuilt.umask_merge_tag = service.umask_merge_tag;
	rebuilt.merge_tags = std::mem::take(&mut service.merge_tags);
	rebuilt.pull_refresh_after_reset = service.pull_refresh_after_reset;
	rebuilt.command_reset = service.command_reset;
	rebuilt.entrypoint_reset = service.entrypoint_reset;
	*service = rebuilt;
}
