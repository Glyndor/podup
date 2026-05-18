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
/// first file wins (earlier entries in the list have higher priority).
/// `env_file:` never overrides service-level `environment:`.
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
                return Err(ComposeError::Unsupported(format!(
                    "env_file format '{fmt}' not supported (only 'dotenv')"
                )));
            }
        }

        let abs = base_dir.join(entry.path());
        let content = match std::fs::read_to_string(&abs) {
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

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let (key, value) = if let Some(eq) = trimmed.find('=') {
                let k = trimmed[..eq].trim().to_string();
                let v = trimmed[eq + 1..].to_string();
                (k, v)
            } else {
                (trimmed.to_string(), String::new())
            };

            if key.is_empty() {
                continue;
            }

            result.entry(key).or_insert(value);
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
    // Start with service env (higher priority), fill gaps from env_file.
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
