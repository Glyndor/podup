//! Compose file parsing, `extends:` resolution, `include:` merging, and
//! topological service ordering.

pub mod types;

mod anchor;
mod diagnostics;
mod extends;
mod include;
mod merge;
mod order;

use std::path::{Path, PathBuf};

use crate::error::{ComposeError, Result};
use crate::substitute;
use types::ComposeFile;

pub use order::{resolve_levels, resolve_order};

/// Parse a compose file from disk, applying variable substitution and
/// resolving `extends:` / `include:` directives.
pub fn parse_file(path: &Path) -> Result<ComposeFile> {
	parse_file_with_env_files(path, &[])
}

/// Like [`parse_file`], additionally loading `env_files` (the global
/// `--env-file` flag) into the variable map used for interpolation. These take
/// effect for the top-level file and any included files; the process
/// environment and a project `.env` still take precedence.
pub fn parse_file_with_env_files(path: &Path, env_files: &[String]) -> Result<ComposeFile> {
	let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
	let dir = abs.parent().unwrap_or(Path::new(".")).to_path_buf();
	let mut file = parse_file_inner_with_env(&abs, &dir, env_files)?;

	let includes = std::mem::take(&mut file.include);
	for inc in includes {
		let (extra_env_files, project_dir_override) = match &inc {
			types::IncludeConfig::Long {
				env_file,
				project_directory,
				..
			} => (
				env_file.as_ref().map(|ef| ef.to_list()).unwrap_or_default(),
				project_directory.as_ref().map(|pd| dir.join(pd)),
			),
			_ => (vec![], None),
		};
		for rel in inc.paths() {
			let rel_path = std::path::Path::new(&rel);
			// The Compose Specification resolves `include` paths relative to the
			// including file and treats `../` as canonical (monorepos routinely use
			// `include: ../shared/compose.yaml`). An absolute path is used as given.
			// This matches docker-compose and the trusted-input policy already
			// applied to `extends.file` and `env_file` — the compose file is
			// trusted input, like a Makefile.
			let inc_path = if rel_path.is_absolute() {
				rel_path.to_path_buf()
			} else {
				dir.join(&rel)
			};
			let inc_dir = project_dir_override.clone().unwrap_or_else(|| {
				inc_path
					.parent()
					.map(|p| p.to_path_buf())
					.unwrap_or_else(|| dir.clone())
			});
			let mut combined_env_files = env_files.to_vec();
			combined_env_files.extend(extra_env_files.iter().cloned());
			let mut included = parse_file_inner_with_env(&inc_path, &inc_dir, &combined_env_files)?;
			anchor::anchor_compose_file(&mut included, &inc_dir);
			include::merge_compose_file(&mut file, included);
		}
	}

	extends::resolve_all_extends(&mut file, &dir)?;
	Ok(file)
}

/// Parse and merge multiple compose files (the `-f`/`COMPOSE_FILE` list).
///
/// Files are merged left to right: a later file overrides an earlier one,
/// service by service (per-field, like `extends`), with top-level
/// volumes/networks/secrets/configs replaced on key conflicts. Relative paths
/// resolve against the first file's directory, matching the compose project
/// directory. `env_files` feed interpolation for every file.
pub fn parse_files_with_env_files(paths: &[PathBuf], env_files: &[String]) -> Result<ComposeFile> {
	let mut iter = paths.iter();
	let first = iter
		.next()
		.ok_or_else(|| ComposeError::FileNotFound("no compose file given".to_string()))?;
	let mut merged = parse_file_with_env_files(first, env_files)?;
	for path in iter {
		let other = parse_file_with_env_files(path, env_files)?;
		merge_override(&mut merged, other);
	}
	for warning in diagnostics::collect(&merged) {
		tracing::warn!("{warning}");
	}
	Ok(merged)
}

/// Merge `other` into `target` with `other` winning (compose `-f` override
/// semantics): services are merged field-by-field, other top-level maps replace
/// on key conflict.
fn merge_override(target: &mut ComposeFile, other: ComposeFile) {
	for (name, svc) in other.services {
		if let Some(base) = target.services.get_mut(&name) {
			*base = extends::merge_service(std::mem::take(base), svc);
		} else {
			target.services.insert(name, svc);
		}
	}
	for (k, v) in other.volumes {
		target.volumes.insert(k, v);
	}
	for (k, v) in other.networks {
		target.networks.insert(k, v);
	}
	for (k, v) in other.secrets {
		target.secrets.insert(k, v);
	}
	for (k, v) in other.configs {
		target.configs.insert(k, v);
	}
}

/// Parse a compose YAML string (no file I/O).
///
/// Variable substitution is applied using only the process environment.
/// `extends: { file: ... }` and `include:` directives are not resolved —
/// use [`parse_file`] for that.
pub fn parse_str(content: &str) -> Result<ComposeFile> {
	let vars = substitute::build_vars(Path::new("."));
	let substituted = substitute::substitute(content, &vars)?;
	let mut file = merge::deserialize_with_merge(&substituted)?;
	extends::resolve_extends_same_file(&mut file)?;
	Ok(file)
}

/// Parse raw (already-substituted) YAML into a `ComposeFile` without any
/// post-processing.
pub fn parse_str_raw(content: &str) -> Result<ComposeFile> {
	merge::deserialize_with_merge(content)
}

pub(crate) fn parse_file_inner(path: &Path, dir: &Path) -> Result<ComposeFile> {
	parse_file_inner_with_env(path, dir, &[])
}

pub(crate) fn parse_file_inner_with_env(
	path: &Path,
	dir: &Path,
	extra_env_files: &[String],
) -> Result<ComposeFile> {
	let content = crate::filesystem::read_to_string_capped(path).map_err(|e| {
		if e.kind() == std::io::ErrorKind::NotFound {
			ComposeError::FileNotFound(path.display().to_string())
		} else {
			ComposeError::Io(e)
		}
	})?;
	let vars = if extra_env_files.is_empty() {
		substitute::build_vars(dir)
	} else {
		substitute::build_vars_with_env_files(dir, extra_env_files)
	};
	let substituted = substitute::substitute(&content, &vars)?;
	merge::deserialize_with_merge(&substituted)
}

#[cfg(test)]
mod tests {
	use super::*;

	// parse_str_raw

	#[test]
	fn parse_str_raw_minimal_service() {
		let yaml = "services:\n  web:\n    image: nginx\n";
		let file = parse_str_raw(yaml).unwrap();
		assert!(file.services.contains_key("web"));
		assert_eq!(file.services["web"].image.as_deref(), Some("nginx"));
	}

	#[test]
	fn parse_str_raw_invalid_yaml_is_error() {
		assert!(parse_str_raw(": : :").is_err());
	}

	// unknown-key capture / warning

	#[test]
	fn unknown_service_key_is_captured_not_dropped() {
		// A typo'd key lands in `unknown` instead of vanishing silently.
		let yaml = "services:\n  web:\n    image: nginx\n    enviroment:\n      - A=1\n";
		let file = parse_str_raw(yaml).unwrap();
		assert!(file.services["web"].unknown.contains_key("enviroment"));
		assert!(file.services["web"].environment.is_empty());
	}

	#[test]
	fn known_service_keys_do_not_land_in_unknown() {
		let yaml = "services:\n  web:\n    image: nginx\n    environment:\n      - A=1\n";
		let file = parse_str_raw(yaml).unwrap();
		assert!(file.services["web"].unknown.is_empty());
	}

	// YAML merge keys (<<)

	#[test]
	fn yaml_merge_key_fills_missing_fields() {
		let yaml = "x-defaults: &defaults\n  image: nginx\n  restart: always\nservices:\n  web:\n    <<: *defaults\n    ports: ['80:80']\n";
		let file = parse_str_raw(yaml).unwrap();
		assert_eq!(file.services["web"].image.as_deref(), Some("nginx"));
	}
}
