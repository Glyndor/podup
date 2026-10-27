//! Scalars that a compose file writes as numbers or booleans in fields podup
//! models as text.
//!
//! `user: 1000`, `ports: [8080]`, `labels: {replicas: 3}` and `mem_limit: 512`
//! are valid compose, and so is a `${PORT}` that interpolates to `8080`. The
//! YAML parser types those as numbers, and a field declared as a string refuses
//! a number, so the whole file failed to parse without naming the field. This
//! turns the scalar back into its text before the document is typed, for the
//! keys that are text, and leaves every key that is really numeric alone.

use serde_yaml::Value;

/// Service keys whose value is one string.
const TEXT_SCALARS: &[&str] = &[
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
	"cgroup",
	"cgroup_parent",
	"userns_mode",
	"uts",
	"isolation",
	"mem_limit",
	"memswap_limit",
	"mem_reservation",
	"shm_size",
	"mac_address",
];

/// Service keys whose value is a string, a list of strings or a map of strings.
const TEXT_COLLECTIONS: &[&str] = &[
	"command",
	"entrypoint",
	"ports",
	"expose",
	"dns",
	"dns_search",
	"dns_opt",
	"cap_add",
	"cap_drop",
	"security_opt",
	"tmpfs",
	"volumes",
	"profiles",
	"devices",
	"extra_hosts",
	"external_links",
	"links",
	"volumes_from",
	"labels",
	"annotations",
];

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
