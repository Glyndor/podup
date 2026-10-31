//! The `!override` and `!reset` merge tags.
//!
//! Compose defines two YAML tags that change how a key merges across `-f` files
//! and `extends`, and they are the documented escape hatch from every merge rule
//! that would otherwise combine rather than replace:
//!
//! ```yaml
//! services:
//!   web:
//!     ports: !override ["9090:80"]   # replace the base's ports, do not append
//!     dns:   !reset []               # drop the base's dns entirely
//! ```
//!
//! podup accepted both and then **ignored them silently**: `!override` still
//! appended and `!reset` still kept the base. Silently doing the opposite of
//! what a key asks for is worse than refusing it, and the tags exist precisely
//! for the cases where the default merge is wrong.
//!
//! The tag is lost when YAML is deserialized into the typed `Service`, which is
//! where merging happens, so it has to be collected from the raw document first.
//! That is what this module does: read which service keys carry which tag, and
//! hand that to the merge as a side channel.

use std::collections::HashMap;

use serde_yaml::Value;

/// What a tag asks the merge to do with one key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MergeTag {
	/// `!override`: take the overriding file's value whole, skipping the
	/// combine rule that would otherwise apply.
	Override,
	/// `!reset`: drop the key entirely, leaving the type's default.
	Reset,
}

/// Tagged keys per service or job name: `services.<name>.<key>` or
/// `jobs.<name>.<key>` → tag. The two sections share one namespace, so the
/// name alone identifies the entry.
pub(crate) type Directives = HashMap<String, HashMap<String, MergeTag>>;

/// The entries of the `services:` and `jobs:` sections of a raw document.
fn definitions(raw: &Value) -> impl Iterator<Item = (&Value, &Value)> {
	["services", "jobs"]
		.into_iter()
		.filter_map(|section| raw.get(section).and_then(Value::as_mapping))
		.flat_map(|section| section.iter())
}

/// The `(section, name)` of every service and job whose `umask` carries
/// `!reset`. The section is part of the identity: a service and a job may be
/// written with one name in a malformed file, and a reset asked for in one must
/// not be applied to the other.
pub(crate) fn umask_resets(raw: &Value) -> Vec<(&'static str, String)> {
	let mut found = Vec::new();
	for section in ["services", "jobs"] {
		let Some(entries) = raw.get(section).and_then(Value::as_mapping) else {
			continue;
		};
		for (name, entry) in entries {
			let tagged = matches!(
				entry.get("umask"),
				Some(Value::Tagged(t)) if t.tag == "!reset"
			);
			if let (true, Some(name)) = (tagged, name.as_str()) {
				found.push((section, name.to_string()));
			}
		}
	}
	found
}

/// Collect the merge tags in a raw compose document.
///
/// Reads the document only for its structure, so interpolation is irrelevant:
/// a tag is attached to a key, never to a value's contents.
///
/// An unrecognized tag is ignored rather than rejected: the compose spec lets a
/// document carry tags this tool does not define, and refusing them would break
/// files that are valid elsewhere.
pub(crate) fn collect(raw: &Value) -> Directives {
	let mut out: Directives = HashMap::new();
	for (name, service) in definitions(raw) {
		let (Some(name), Some(fields)) = (name.as_str(), service.as_mapping()) else {
			continue;
		};
		let mut tagged: HashMap<String, MergeTag> = HashMap::new();
		for (key, value) in fields {
			let (Some(key), Value::Tagged(t)) = (key.as_str(), value) else {
				continue;
			};
			let tag = match t.tag.to_string().as_str() {
				"!override" => MergeTag::Override,
				"!reset" => MergeTag::Reset,
				_ => continue,
			};
			tagged.insert(key.to_string(), tag);
		}
		if !tagged.is_empty() {
			out.insert(name.to_string(), tagged);
		}
	}
	collect_trigger_tags(raw, &mut out);
	out
}

/// Tags inside a job's `triggers:`, recorded as `triggers.manual` and
/// `triggers.schedule` beside the job's own keys. `triggers` itself is
/// collected with the other keys of the job.
fn collect_trigger_tags(raw: &Value, out: &mut Directives) {
	let Some(jobs) = raw.get("jobs").and_then(Value::as_mapping) else {
		return;
	};
	for (name, job) in jobs {
		let Some(name) = name.as_str() else { continue };
		let triggers = match job.get("triggers") {
			Some(Value::Tagged(t)) => &t.value,
			Some(other) => other,
			None => continue,
		};
		let Some(fields) = triggers.as_mapping() else {
			continue;
		};
		for (key, value) in fields {
			let (Some(key), Value::Tagged(t)) = (key.as_str(), value) else {
				continue;
			};
			let tag = match t.tag.to_string().as_str() {
				"!override" => MergeTag::Override,
				"!reset" => MergeTag::Reset,
				_ => continue,
			};
			out.entry(name.to_string())
				.or_default()
				.insert(format!("triggers.{key}"), tag);
		}
	}
}

/// The merge tags of one service or job, ready to travel with it.
///
/// The keys that carry their own channel (`umask`, `pull_refresh_after`) are
/// left out: they are decided while the document is typed, and repeating them
/// here would apply the same directive twice.
pub(crate) fn service_tags(directives: &Directives, name: &str) -> HashMap<String, MergeTag> {
	let mut tags = directives.get(name).cloned().unwrap_or_default();
	tags.remove("umask");
	tags.remove("pull_refresh_after");
	tags
}

/// Remove every `!override`/`!reset` tag from a document, keeping the value it
/// wraps.
///
/// This has to run before the document is deserialized into typed structs.
/// Whether a tag is tolerated otherwise depends entirely on the field's Rust
/// type: `ports` is a `Vec`, which quietly ignores it, but `dns` is an untagged
/// enum, and serde refuses those outright, so `dns: !reset []` failed the whole
/// file with "failed to parse compose file" while `ports: !reset []` parsed and
/// did nothing. Two different wrong behaviours for the same tag, decided by an
/// implementation detail the user cannot see.
///
/// Stripping first makes the tag mean the same thing everywhere; what it *does*
/// is then decided by [`collect`] and the merge, not by serde.
pub(crate) fn strip(value: &mut Value) {
	match value {
		Value::Tagged(t) => {
			let mut inner = std::mem::replace(&mut t.value, Value::Null);
			strip(&mut inner);
			*value = inner;
		}
		Value::Mapping(map) => {
			// A tag can sit on a key's value at any depth, and rebuilding the map
			// preserves insertion order, which the compose output relies on.
			let entries: Vec<(Value, Value)> = std::mem::take(map).into_iter().collect();
			for (k, mut v) in entries {
				strip(&mut v);
				map.insert(k, v);
			}
		}
		Value::Sequence(seq) => {
			for v in seq.iter_mut() {
				strip(v);
			}
		}
		_ => {}
	}
}

/// Read a compose file's merge tags, or an empty set when it cannot be read or
/// parsed.
///
/// Best-effort by design: this runs alongside the real parse, which reports any
/// genuine syntax error with a proper message. Failing here too would only
/// duplicate that, and returning empty simply means "no tags", which is the
/// behaviour every file without them already gets.
pub(crate) fn collect_from_file(path: &std::path::Path) -> Directives {
	let Ok(text) = std::fs::read_to_string(path) else {
		return Directives::new();
	};
	match serde_yaml::from_str::<Value>(&text) {
		Ok(raw) => collect(&raw),
		Err(_) => Directives::new(),
	}
}

/// Preserve explicit null and reset intent for the refresh extension through typing.
pub(crate) fn refresh_clears(raw: &Value) -> Vec<String> {
	definitions(raw)
		.filter_map(|(name, service)| {
			let value = service.get("pull_refresh_after")?;
			let clear = match value {
				Value::Null => true,
				Value::Tagged(t) => {
					t.tag == "!reset" || (t.tag == "!override" && t.value.is_null())
				}
				_ => false,
			};
			clear.then(|| name.as_str().map(str::to_owned)).flatten()
		})
		.collect()
}

#[cfg(test)]
#[path = "tags_tests.rs"]
mod tests;
