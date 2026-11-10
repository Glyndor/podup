//! The two spellings of a `devices:` entry: the short `host[:container[:perms]]`
//! string and the long `{source, target, permissions}` mapping.

use serde::{Deserialize, Deserializer};

/// One `devices:` entry as written.
#[derive(Deserialize)]
#[serde(untagged)]
enum Entry {
	Short(String),
	Long(Long),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Long {
	source: String,
	target: Option<String>,
	permissions: Option<String>,
}

impl Entry {
	/// The short form every later stage reads. A long entry names its target
	/// explicitly, as Docker requires, and its permissions only when given.
	fn into_short<E: serde::de::Error>(self, index: usize) -> Result<String, E> {
		let Long {
			source,
			target,
			permissions,
		} = match self {
			Entry::Short(text) => return Ok(text),
			Entry::Long(long) => long,
		};
		let Some(target) = target.filter(|t| !t.is_empty()) else {
			return Err(E::custom(format!(
				"service device devices[{index}] is missing a mount target"
			)));
		};
		Ok(match permissions.filter(|p| !p.is_empty()) {
			Some(permissions) => format!("{source}:{target}:{permissions}"),
			None => format!("{source}:{target}"),
		})
	}
}

/// Read a `devices:` list in either spelling into the short strings.
pub(super) fn deserialize<'de, D>(de: D) -> Result<Vec<String>, D::Error>
where
	D: Deserializer<'de>,
{
	Vec::<Entry>::deserialize(de)?
		.into_iter()
		.enumerate()
		.map(|(index, entry)| entry.into_short(index))
		.collect()
}

/// The path a device appears at inside the container: the second segment of
/// the short form, or the host path when the entry names no other.
pub(crate) fn container_path(entry: &str) -> &str {
	let mut parts = entry.splitn(3, ':');
	let host = parts.next().unwrap_or("");
	parts.next().filter(|c| !c.is_empty()).unwrap_or(host)
}

#[cfg(test)]
#[path = "devices_tests.rs"]
mod tests;
