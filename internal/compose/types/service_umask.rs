//! Octal string parsing and service-aware errors for the initial process mask.

use serde::{Deserialize, Deserializer, Serializer};
use serde_yaml::Value;

use crate::error::Result;
use crate::libpod::validate::spec_field_error;

/// Preserve strings and integer zero; other integers have ambiguous radix.
pub(super) fn deserialize<'de, D>(de: D) -> std::result::Result<Option<String>, D::Error>
where
	D: Deserializer<'de>,
{
	use serde::de::Error;
	match Value::deserialize(de)? {
		Value::Null => Ok(None),
		Value::Number(n) if n.as_u64() == Some(0) => Ok(Some("0".into())),
		Value::Number(_) => Err(D::Error::custom(
			"umask must be a quoted octal string; quote the value (only integer 0 is accepted)",
		)),
		Value::String(s) => {
			if !valid(&s) {
				return Err(D::Error::custom(diagnostic(&s)));
			}
			Ok(Some(s))
		}
		_ => Err(D::Error::custom(
			"umask must be a quoted octal string or integer 0",
		)),
	}
}

/// Write the mask back as the octal string it was read from, so the output of
/// `podup config` loads again and means the same mask.
pub(super) fn serialize<S: Serializer>(
	mask: &Option<String>,
	out: S,
) -> std::result::Result<S::Ok, S::Error> {
	match mask {
		Some(text) if valid(text) => out.serialize_str(&normalized(text)),
		Some(text) => Err(serde::ser::Error::custom(diagnostic(text))),
		None => out.serialize_none(),
	}
}

/// Match libpod's one-to-four octal digit range without losing written text.
fn valid(text: &str) -> bool {
	(1..=4).contains(&text.len()) && text.bytes().all(|b| matches!(b, b'0'..=b'7'))
}

/// Pad valid masks for output while preserving unresolved interpolation text.
/// Invalid text is refused by parsing, serialization and each runtime caller.
/// There is no unresolved-text output path; callers supply validated digits.
pub(crate) fn normalized(text: &str) -> String {
	format!("{text:0>4}")
}

/// Check the final text before any existing container can be removed.
pub(crate) fn validate_runtime(service: &str, text: &str) -> Result<()> {
	if !valid(text) {
		return Err(spec_field_error(service, "umask", text, diagnostic(text)).into());
	}
	Ok(())
}

/// Validate before typing the document so errors retain the owning service name.
pub(crate) fn validate(value: &Value) -> Result<()> {
	for section in ["services", "jobs"] {
		let Some(entries) = value.get(section).and_then(Value::as_mapping) else {
			continue;
		};
		for (name, service) in entries {
			if let Some(raw) = service.get("umask") {
				if section == "jobs" {
					// A job's error names the job and does not repeat the value
					// the file gave.
					if deserialize(raw.clone()).is_err() {
						return Err(crate::error::ComposeError::Job(format!(
							"job '{}' has an invalid `umask`; it must be a quoted octal string \
							 of one to four digits (0 to 7)",
							crate::error::sanitize_name(name.as_str().unwrap_or_default())
						)));
					}
					continue;
				}
				deserialize(raw.clone()).map_err(|error| {
					spec_field_error(
						name.as_str().unwrap_or_default(),
						"umask",
						format!("{raw:?}"),
						error.to_string(),
					)
				})?;
			}
		}
	}
	Ok(())
}

fn diagnostic(text: &str) -> &'static str {
	if text.contains('$') {
		"umask must contain one to four octal digits (0 to 7); quote the value; variable did not resolve"
	} else {
		"umask must contain one to four octal digits (0 to 7); quote the value"
	}
}

/// Reset is effective even when a service never participates in a merge.
///
/// `resets` names the entries by section and name, from
/// [`crate::compose::tags::umask_resets`], read before the tags were stripped.
/// An entry that is not a mapping is left as it is: its own parse reports it,
/// and indexing into it here would abort the process.
pub(crate) fn apply_resets(value: &mut Value, resets: &[(&'static str, String)]) {
	for (section, name) in resets {
		let entry = value
			.get_mut(*section)
			.and_then(|entries| entries.get_mut(name.as_str()));
		if let Some(map) = entry.and_then(Value::as_mapping_mut) {
			map.insert(Value::from("umask"), Value::Null);
		}
	}
}

#[path = "service_umask_number.rs"]
mod number;
pub(crate) use number::number_error;
