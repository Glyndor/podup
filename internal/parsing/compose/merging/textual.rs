//! Scalars that a compose file writes as numbers or booleans in fields podup
//! models as text.
//!
//! `ports: [8080]`, `expose: [3000]`, `labels: {replicas: 3}` and
//! `mem_limit: 512` are valid compose: the schema types them "string or
//! number". The YAML parser types them as numbers, and a field declared as a
//! string refuses a number, so the whole file failed to parse without naming
//! the field. This turns the scalar back into its text before the document is
//! typed. A key the schema types as a plain string, like `user`, keeps
//! rejecting a bare number.

use serde_yaml::Value;

/// Service keys the compose schema types as a plain string (or a list of
/// strings). A bare number written there is a type error and stays one; only
/// an interpolated value is kept as text, see [`is_string_key`].
const STRING_KEYS: &[&str] = &[
	"command",
	"entrypoint",
	"user",
	"container_name",
	"hostname",
	"domainname",
	"working_dir",
	"platform",
	"pid",
	"ipc",
	"network_mode",
	"restart",
	"stop_signal",
	"pull_policy",
	"runtime",
	"cgroup_parent",
	"userns_mode",
	"uts",
	"isolation",
	"mac_address",
];

/// Whether the interpolated value under `key` is text (see [`STRING_KEYS`]).
pub(super) fn is_string_key(key: &str) -> bool {
	STRING_KEYS.contains(&key)
}

/// Service keys whose schema type is "string or number": a bare number is valid
/// compose there and loads as its text.
const TEXT_SCALARS: &[&str] = &["mem_limit", "memswap_limit", "mem_reservation", "shm_size"];

/// Service keys that hold a list of "string or number" items, or a map whose
/// values are string, number or boolean.
const TEXT_COLLECTIONS: &[&str] = &["ports", "expose", "labels", "annotations"];

/// Turn the number and boolean scalars of the text-typed keys of every service
/// and job in `document` into strings.
pub(in super::super) fn stringify_text_fields(document: &mut Value) {
	for section in ["services", "jobs"] {
		let Some(Value::Mapping(entries)) = document.get_mut(section) else {
			continue;
		};
		for (_, entry) in entries.iter_mut() {
			let Value::Mapping(service) = entry else {
				continue;
			};
			for (key, value) in service.iter_mut() {
				let Some(key) = key.as_str() else { continue };
				if TEXT_SCALARS.contains(&key) {
					stringify_scalar(value);
				} else if TEXT_COLLECTIONS.contains(&key) {
					stringify_collection(value);
				}
			}
		}
	}
}

fn stringify_scalar(value: &mut Value) {
	match value {
		Value::Number(n) => *value = Value::String(n.to_string()),
		Value::Bool(b) => *value = Value::String(b.to_string()),
		_ => {}
	}
}

/// A string, the elements of a list or the values of a map. Elements that are
/// themselves lists or maps (a long-form port or volume) are left as they are.
fn stringify_collection(value: &mut Value) {
	match value {
		Value::Sequence(items) => items.iter_mut().for_each(stringify_scalar),
		Value::Mapping(map) => map.iter_mut().for_each(|(_, v)| stringify_scalar(v)),
		other => stringify_scalar(other),
	}
}

#[cfg(test)]
#[path = "textual_tests.rs"]
mod tests;
