//! `env_file:` loading for services.
//!
//! Reads KEY=VALUE pairs from files listed in a service's `env_file:` field.
//! Service-level `environment:` takes precedence over `env_file:` values.

use std::collections::HashMap;
use std::path::Path;

use crate::compose::types::EnvFileEntry;
use crate::error::{ComposeError, Result};

/// Load all `env_file` paths relative to `base_dir`.
///
/// Returns a merged map.  If the same key appears in multiple files, the
/// last file wins (later entries in the list override earlier ones).
/// `env_file:` never overrides service-level `environment:`.
///
/// Each file is parsed with dotenv rules (quote stripping, escapes, inline
/// comments, multi-line quoted values). Unquoted and double-quoted values are
/// then interpolated, as the Compose Specification says and Docker Compose
/// does: `${VAR}`, `$VAR`, `${VAR:-default}` and `$$` are resolved against the
/// project environment (the process environment and the project's `.env`), and
/// then against the keys the env files before them in the list, and this one
/// above the value, set. A single-quoted value stays literal. An entry written with
/// `format: raw` is read line by line, every value kept as written and none
/// interpolated.
///
/// Returns [`ComposeError::FileNotFound`] when an env file does not exist.
pub fn load_env_files(paths: &[String], base_dir: &Path) -> Result<HashMap<String, String>> {
	let entries: Vec<EnvFileEntry> = paths
		.iter()
		.map(|p| EnvFileEntry::Path(p.clone()))
		.collect();
	load_env_file_entries(&entries, base_dir)
}

/// Load env_file entries supporting both short and long-form (with `required` and `format`).
///
/// When `required: false`, a missing file is silently skipped instead of returning an error.
pub fn load_env_file_entries(
	entries: &[EnvFileEntry],
	base_dir: &Path,
) -> Result<HashMap<String, String>> {
	load_env_file_entries_for(entries, base_dir, &HashMap::new())
}

/// As [`load_env_file_entries`], for the files of a service whose own
/// `environment:` is `service_env`. A value can refer to the project
/// environment first, then to what the service's `environment:` sets, then to
/// the keys the files set, which is the order Docker Compose looks them up in.
pub fn load_env_file_entries_for(
	entries: &[EnvFileEntry],
	base_dir: &Path,
	service_env: &HashMap<String, Option<String>>,
) -> Result<HashMap<String, String>> {
	let mut result: HashMap<String, String> = HashMap::new();
	// What a value can refer to. The project environment wins over the service's
	// `environment:`, and that over a file's own keys, as it does in Docker Compose.
	let mut scope = crate::substitute::build_vars(base_dir);
	for (key, value) in service_env {
		if let Some(value) = value {
			scope.entry(key.clone()).or_insert_with(|| value.clone());
		}
	}
	// One budget and one set of warned names for every value of every file in
	// this call, as for a compose document: many values that each expand a large
	// variable are bounded together, not one by one.
	// When a project scope is open (`config`, over every service), they all draw on
	// one total, as the files of a compose document do.
	let mut spent = crate::substitute::project_budget::spent();
	let mut warned = std::collections::HashSet::new();

	for entry in entries {
		let format = match entry {
			EnvFileEntry::Config {
				format: Some(fmt), ..
			} => fmt.as_str(),
			_ => "dotenv",
		};
		if !matches!(format, "dotenv" | "raw") {
			// compose-go logs a warning and falls back to dotenv parsing
			// rather than failing the file; match that lenient behaviour.
			tracing::warn!(
				"env_file format '{format}' is not supported; parsing '{}' as dotenv",
				entry.path()
			);
		}

		let abs = base_dir.join(entry.path());
		let content = match crate::filesystem::read_to_string_capped(&abs) {
			Ok(c) => c,
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
				if entry.required() {
					return Err(ComposeError::FileNotFound(abs.display().to_string()));
				} else {
					continue;
				}
			}
			// Name the file: a bare "Permission denied (os error 13)" does not say
			// which of the files of the project it is.
			Err(e) => {
				return Err(ComposeError::EnvFile(format!(
					"cannot read env file {}: {e}",
					abs.display()
				)))
			}
		};

		// A service `env_file:` is explicitly requested, so a malformed entry
		// (e.g. an unterminated quoted value that would otherwise swallow the
		// following keys) is a hard error rather than silent data loss.
		let pairs: Vec<(String, String, bool)> = if format == "raw" {
			// Raw: every value as written, so none is interpolated.
			crate::dotenv::parse_raw(&content)?
				.into_iter()
				.map(|(key, value)| (key, value, true))
				.collect()
		} else {
			crate::dotenv::parse_strict_marked(&content)?
		};
		for (key, value, literal) in pairs {
			let value = if literal {
				value
			} else {
				crate::substitute::substitute_budgeted(&value, &scope, &mut spent, &mut warned)?
			};
			scope.entry(key.clone()).or_insert_with(|| value.clone());
			result.insert(key, value);
		}
	}

	crate::substitute::project_budget::record(spent);
	Ok(result)
}

/// Merge env_file values with service environment.
///
/// `service_env` takes precedence: only keys not already in `service_env` are added.
pub fn merge_env(
	service_env: HashMap<String, Option<String>>,
	env_file_vars: HashMap<String, String>,
) -> Vec<String> {
	let mut merged = service_env;
	for (k, v) in env_file_vars {
		merged.entry(k).or_insert(Some(v));
	}

	merged
		.into_iter()
		.map(|(k, v)| match v {
			Some(val) => format!("{k}={val}"),
			None => k,
		})
		.collect()
}

/// Give every bare `KEY` of every service's `environment:` the value the
/// project environment holds for it, so `config` prints what the container
/// will run with. A key the project environment does not define stays bare and
/// renders as `KEY: null`, as in docker compose. Run it before
/// [`materialize_env_files`] so a file value can refer to the resolved key.
pub fn resolve_bare_environment(
	file: &mut crate::compose::types::ComposeFile,
	vars: &HashMap<String, String>,
) {
	for service in file.services_and_jobs_mut() {
		service.environment.resolve_bare(vars);
	}
}

/// Fold every service's `env_file:` into its `environment:` and drop the key.
///
/// `config` is meant to render the canonical, fully-resolved model, and docker
/// compose materialises `env_file` there. Leaving it unresolved meant a service
/// that takes its whole environment from a file rendered with no `environment:`
/// at all, so the one command you reach for to ask "what will this actually run"
/// pointed away from the answer rather than merely omitting it (#1184).
///
/// Precedence is the same as at run time: `environment:` wins over `env_file:`,
/// and a later file wins over an earlier one. Keys are emitted sorted so the
/// output is stable across runs rather than following a hash map's order.
///
/// A bare `KEY` (inherit from the host) stays valueless, rendering as `KEY: null`
/// the way the parser accepts it back.
pub fn materialize_env_files(
	file: &mut crate::compose::types::ComposeFile,
	base_dir: &Path,
) -> Result<()> {
	// One interpolation budget for the env files of every service.
	let _budget = crate::substitute::project_budget::Scope::open();
	for service in file.services_and_jobs_mut() {
		materialize_service_env_files(service, base_dir)?;
	}
	Ok(())
}

/// Fold one effective container's env files into its explicit environment.
/// Call after inheritance so file values do not become explicit overrides
/// before a pre-start step is merged. Explicit keys win, including bare keys.
#[doc(hidden)]
pub fn materialize_service_env_files(
	service: &mut crate::compose::types::Service,
	base_dir: &Path,
) -> Result<()> {
	let entries = service.env_file.to_entries();
	if entries.is_empty() {
		return Ok(());
	}
	let mut merged = service.environment.to_map();
	let from_files = load_env_file_entries_for(&entries, base_dir, &merged)?;
	for (k, v) in from_files {
		merged.entry(k).or_insert(Some(v));
	}
	let mut keys: Vec<String> = merged.keys().cloned().collect();
	keys.sort();
	let map = keys
		.into_iter()
		.map(|k| {
			let value = merged
				.get(&k)
				.and_then(|v| v.clone())
				.map(serde_yaml::Value::String);
			(k, value)
		})
		.collect();
	service.environment = crate::compose::types::EnvVars::Map(map);
	service.env_file = crate::compose::types::EnvFile::Empty;
	Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "env_file_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "env_file_interpolation_tests.rs"]
mod interpolation_tests;

#[cfg(test)]
#[path = "env_file_raw_tests.rs"]
mod raw_tests;
