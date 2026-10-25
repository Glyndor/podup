//! Parsing a `jobs:` map from raw YAML into typed jobs.

use indexmap::IndexMap;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use serde_yaml::Value;

use super::triggers;
use crate::compose::types::{Job, Service, Triggers};
use crate::error::{sanitize_name, ComposeError, Result};

/// Build a job error. The text is composed from escaped names and fixed
/// words only: it never echoes a value from the file.
pub(super) fn job_error(message: String) -> ComposeError {
	ComposeError::Job(message)
}

/// A job depends on a name that is neither a service nor a job.
pub(super) fn dependency_error(job: &str, dependency: &str) -> ComposeError {
	job_error(format!(
		"job '{}' depends on '{}', which is not defined as a service or a job",
		sanitize_name(job),
		sanitize_name(dependency)
	))
}

/// Type one job from its raw YAML.
///
/// `triggers` is taken out first and checked by [`triggers::parse`]; what is
/// left is a service definition. The service error is deliberately terse:
/// serde's message embeds the offending scalar, which is the file's own text.
pub(crate) fn parse_job(name: &str, raw: Value) -> Result<Job> {
	let label = sanitize_name(name);
	let Value::Mapping(mut map) = raw else {
		return Err(job_error(format!(
			"job '{label}' must be a mapping of keys, like a service"
		)));
	};
	// A missing `triggers` is allowed here: a file merged over another may leave
	// the triggers to the first one. `check_triggers_declared` requires them of
	// the merged file.
	let triggers = match map.remove("triggers") {
		None | Some(Value::Null) => Triggers::default(),
		Some(raw) => triggers::parse(&label, &raw)?,
	};
	let service: Service = serde_yaml::from_value(Value::Mapping(map)).map_err(|_| {
		job_error(format!(
			"job '{label}' is not a valid job definition; check the type of each of its keys"
		))
	})?;
	Ok(Job { service, triggers })
}

/// Type a whole `jobs:` map, in file order.
fn parse_jobs(raw: IndexMap<String, Value>) -> Result<IndexMap<String, Job>> {
	let mut jobs = IndexMap::with_capacity(raw.len());
	for (name, value) in raw {
		let job = parse_job(&name, value)?;
		jobs.insert(name, job);
	}
	Ok(jobs)
}

/// `deserialize_with` for `ComposeFile::jobs`: an absent key, `jobs:` and
/// `jobs: {}` all declare nothing.
pub(crate) fn deserialize_jobs<'de, D>(
	deserializer: D,
) -> std::result::Result<IndexMap<String, Job>, D::Error>
where
	D: Deserializer<'de>,
{
	let raw = Option::<IndexMap<String, Value>>::deserialize(deserializer)?;
	parse_jobs(raw.unwrap_or_default()).map_err(D::Error::custom)
}

/// Take the `jobs:` section out of a raw compose document and type it.
///
/// Done before the document reaches `ComposeFile`'s own deserializer so a
/// failure names the job and says what is wrong, where the generic path can
/// only report that the file did not parse.
pub(in crate::compose) fn extract_jobs(document: &mut Value) -> Result<IndexMap<String, Job>> {
	let Some(root) = document.as_mapping_mut() else {
		return Ok(IndexMap::new());
	};
	let raw = match root.remove("jobs") {
		None | Some(Value::Null) => return Ok(IndexMap::new()),
		Some(Value::Mapping(map)) => map,
		Some(_) => {
			return Err(job_error(
				"the top-level `jobs:` key must be a mapping of job names to definitions"
					.to_string(),
			))
		}
	};
	let mut jobs = IndexMap::with_capacity(raw.len());
	for (key, value) in raw {
		let Value::String(name) = key else {
			return Err(job_error(
				"a job name must be a string; quote names that YAML reads as numbers".to_string(),
			));
		};
		let job = parse_job(&name, value)?;
		jobs.insert(name, job);
	}
	Ok(jobs)
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
