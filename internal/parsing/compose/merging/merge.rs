//! YAML merge-key (`<<:`) and anchor resolution ahead of typing a compose file.
//!
//! serde's tolerance of a merge key depends on the type behind it, so the tags
//! are resolved on the raw `Value` first and the merged document is what gets
//! deserialized; the type never sees a `<<:` key.

use crate::compose::types::ComposeFile;
use crate::error::Result;
use std::collections::{HashMap, HashSet};

mod guards;
mod interpolation;
mod keys;

use guards::{guard_alias_expansion, guard_flow_depth};
use interpolation::interpolate_value;
use keys::apply_merge_keys;

#[cfg(test)]
use guards::{count_alias_refs, MAX_ALIAS_DOC_BYTES, MAX_ALIAS_REFS, MAX_FLOW_DEPTH};
#[cfg(test)]
use interpolation::value_needs_interp;

pub(in super::super) fn deserialize_with_merge(content: &str) -> Result<ComposeFile> {
	deserialize_with_merge_interp(content, None)
}

/// Parse `content` into a [`ComposeFile`], optionally interpolating `${VAR}`
/// references at the scalar level once the YAML document has been parsed.
///
/// Interpolating *after* parsing (rather than on the raw text) is deliberate:
/// the resolved value of a variable is stored verbatim into the existing scalar
/// node, so an env value containing newlines or YAML syntax is treated as data,
/// never as document structure (no key/`privileged: true` injection), an
/// unset/empty variable always yields an in-place empty string (it cannot drop a
/// key or trigger a "mapping values are not allowed" parse error), and multiline
/// or backslash-bearing values are not re-interpreted by the YAML parser.
pub(in super::super) fn deserialize_with_merge_interp(
	content: &str,
	vars: Option<&HashMap<String, String>>,
) -> Result<ComposeFile> {
	let mut value = interpolated_value(content, vars)?;
	let refresh_clears = super::super::tags::refresh_clears(&value);
	// Drop the merge tags before typing: whether serde tolerates one depends on
	// the field's Rust type, so leaving them in makes `!reset` fail the file on
	// `dns` and do nothing on `ports`. What each tag means is decided by the
	// merge (see `compose::tags`), not by which type happens to be behind a key.
	let directives = super::super::tags::collect(&value);
	let umask_resets = super::super::tags::umask_resets(&value);
	super::super::tags::strip(&mut value);
	super::super::types::service::umask::apply_resets(&mut value, &umask_resets);
	super::super::types::service::umask::validate(&value)?;
	super::textual::stringify_text_fields(&mut value);
	// Jobs are typed first and on their own, so a bad job is reported by name
	// instead of as a file that failed to parse.
	let jobs = super::super::jobs::extract_jobs(&mut value)?;
	let mut file: ComposeFile = serde_yaml::from_value(value)?;
	file.jobs = jobs;
	for (name, service) in &mut file.services {
		service.umask_merge_tag = directives
			.get(name)
			.and_then(|keys| keys.get("umask"))
			.copied();
		service.merge_tags = super::super::tags::service_tags(&directives, name);
	}
	for (name, job) in &mut file.jobs {
		job.service.umask_merge_tag = directives
			.get(name)
			.and_then(|keys| keys.get("umask"))
			.copied();
		job.service.merge_tags = super::super::tags::service_tags(&directives, name);
	}
	for name in refresh_clears {
		if let Some(service) = file.services.get_mut(&name) {
			service.pull_refresh_after = None;
			service.pull_refresh_after_reset = true;
		} else if let Some(job) = file.jobs.get_mut(&name) {
			job.service.pull_refresh_after = None;
			job.service.pull_refresh_after_reset = true;
		}
	}
	Ok(file)
}

/// Produce the interpolated, merge-key-resolved YAML `Value` for `content`, the
/// exact transformation [`deserialize_with_merge_interp`] applies before it
/// deserializes into a [`ComposeFile`], stopping one step short.
///
/// The raw nested-key diagnostic needs this post-interpolation document shape:
/// the typed `ComposeFile` has already dropped unknown keys nested inside option
/// blocks, so the diagnostic must diff against the raw document the parser saw.
pub(in super::super) fn interpolated_value(
	content: &str,
	vars: Option<&HashMap<String, String>>,
) -> Result<serde_yaml::Value> {
	guard_flow_depth(content)?;
	guard_alias_expansion(content)?;
	let mut value: serde_yaml::Value = serde_yaml::from_str(content).map_err(|error| {
		super::super::types::service::umask::number_error(content).unwrap_or(error.into())
	})?;
	// Merge keys first: a service built from `<<: *base` has its keys only after
	// they are resolved, and the typing of an interpolated value depends on the
	// key being a direct key of the service.
	apply_merge_keys(&mut value);
	if let Some(vars) = vars {
		// One budget for the whole document. The per-scalar cap cannot see
		// repetition spread across scalars, which is the shape that reached
		// 5.27 GB with no single substitution over 1 MiB.
		let mut spent = 0usize;
		// One dedup set for the whole pass: three references to `${X}` in
		// the same document share one warning, not three.
		let mut warned = HashSet::new();
		interpolate_value(&mut value, vars, &mut spent, &mut warned)?;
	}
	Ok(value)
}

#[cfg(test)]
#[path = "merge_tests.rs"]
mod tests;
