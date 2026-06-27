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
/// comments, multi-line quoted values).
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
	let mut result: HashMap<String, String> = HashMap::new();

	for entry in entries {
		if let EnvFileEntry::Config {
			format: Some(fmt), ..
		} = entry
		{
			if fmt != "dotenv" {
				// compose-go logs a warning and falls back to dotenv parsing
				// rather than failing the file; match that lenient behaviour.
				tracing::warn!(
					"env_file format '{fmt}' is not supported; parsing '{}' as dotenv",
					entry.path()
				);
			}
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
			Err(e) => return Err(ComposeError::Io(e)),
		};

		for (key, value) in crate::dotenv::parse(&content) {
			result.insert(key, value);
		}
	}

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

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;
	use crate::compose::types::EnvFileEntry;

	// load_env_file_entries

	#[test]
	fn loads_key_value_pairs() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join(".env"), "FOO=bar\nBAZ=qux\n").unwrap();
		let entries = vec![EnvFileEntry::Path(".env".into())];
		let m = load_env_file_entries(&entries, dir.path()).unwrap();
		assert_eq!(m.get("FOO").map(|s| s.as_str()), Some("bar"));
		assert_eq!(m.get("BAZ").map(|s| s.as_str()), Some("qux"));
	}

	#[test]
	fn skips_comments_and_blank_lines() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join(".env"), "# comment\n\nFOO=bar\n").unwrap();
		let entries = vec![EnvFileEntry::Path(".env".into())];
		let m = load_env_file_entries(&entries, dir.path()).unwrap();
		assert_eq!(m.len(), 1);
	}

	#[test]
	fn bare_key_passes_through_host_or_is_omitted() {
		// A bare key (no `=`) takes its value from the host environment; absent
		// from the host, it is omitted rather than set to an empty string.
		std::env::set_var("PODUP_ENVFILE_BARE_PRESENT", "h");
		std::env::remove_var("PODUP_ENVFILE_BARE_ABSENT");
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(
			dir.path().join(".env"),
			"PODUP_ENVFILE_BARE_PRESENT\nPODUP_ENVFILE_BARE_ABSENT\n",
		)
		.unwrap();
		let entries = vec![EnvFileEntry::Path(".env".into())];
		let m = load_env_file_entries(&entries, dir.path()).unwrap();
		assert_eq!(
			m.get("PODUP_ENVFILE_BARE_PRESENT").map(|s| s.as_str()),
			Some("h")
		);
		assert!(!m.contains_key("PODUP_ENVFILE_BARE_ABSENT"));
		std::env::remove_var("PODUP_ENVFILE_BARE_PRESENT");
	}

	#[test]
	fn last_file_wins_on_duplicate_key() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("a.env"), "FOO=first\n").unwrap();
		std::fs::write(dir.path().join("b.env"), "FOO=second\n").unwrap();
		let entries = vec![
			EnvFileEntry::Path("a.env".into()),
			EnvFileEntry::Path("b.env".into()),
		];
		let m = load_env_file_entries(&entries, dir.path()).unwrap();
		assert_eq!(m.get("FOO").map(|s| s.as_str()), Some("second"));
	}

	#[test]
	fn missing_required_file_returns_error() {
		let dir = tempfile::tempdir().unwrap();
		let entries = vec![EnvFileEntry::Path("nonexistent.env".into())];
		assert!(load_env_file_entries(&entries, dir.path()).is_err());
	}

	#[test]
	fn missing_optional_file_skipped() {
		let dir = tempfile::tempdir().unwrap();
		let entries = vec![EnvFileEntry::Config {
			path: "nonexistent.env".into(),
			required: Some(false),
			format: None,
		}];
		let m = load_env_file_entries(&entries, dir.path()).unwrap();
		assert!(m.is_empty());
	}

	#[test]
	fn non_dotenv_format_warns_and_parses_as_dotenv() {
		// compose-go logs a warning for an unknown `format` and falls back to
		// dotenv parsing rather than failing; podup must not error here.
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join(".env"), "FOO=bar\n").unwrap();
		let entries = vec![EnvFileEntry::Config {
			path: ".env".into(),
			required: Some(false),
			format: Some("json".into()),
		}];
		let m = load_env_file_entries(&entries, dir.path()).unwrap();
		assert_eq!(m.get("FOO").map(|s| s.as_str()), Some("bar"));
	}

	#[test]
	fn loads_parent_relative_env_file() {
		// docker-compose, podman, and podman-compose all accept env_file paths
		// outside the project directory (e.g. a shared `../secrets/.env` in a
		// monorepo); podup must too.
		let root = tempfile::tempdir().unwrap();
		std::fs::write(root.path().join("shared.env"), "FOO=bar\n").unwrap();
		let project = root.path().join("project");
		std::fs::create_dir(&project).unwrap();
		let entries = vec![EnvFileEntry::Path("../shared.env".into())];
		let m = load_env_file_entries(&entries, &project).unwrap();
		assert_eq!(m.get("FOO").map(|s| s.as_str()), Some("bar"));
	}

	// merge_env

	#[test]
	fn service_env_wins_over_file_env() {
		let service_env: HashMap<String, Option<String>> =
			[("FOO".to_string(), Some("from-service".to_string()))].into();
		let file_env: HashMap<String, String> =
			[("FOO".to_string(), "from-file".to_string())].into();
		let result = merge_env(service_env, file_env);
		let foo_entry = result
			.iter()
			.find(|s| s.starts_with("FOO="))
			.unwrap()
			.clone();
		assert_eq!(foo_entry, "FOO=from-service");
	}

	#[test]
	fn file_env_fills_missing_keys() {
		let service_env: HashMap<String, Option<String>> = HashMap::new();
		let file_env: HashMap<String, String> = [("BAR".to_string(), "baz".to_string())].into();
		let result = merge_env(service_env, file_env);
		assert!(result.iter().any(|s| s == "BAR=baz"));
	}

	#[test]
	fn key_only_env_var_has_no_equals() {
		let service_env: HashMap<String, Option<String>> =
			[("PASSTHROUGH".to_string(), None)].into();
		let result = merge_env(service_env, HashMap::new());
		assert!(result.iter().any(|s| s == "PASSTHROUGH"));
	}
}
