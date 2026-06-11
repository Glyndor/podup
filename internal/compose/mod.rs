//! Compose file parsing, `extends:` resolution, `include:` merging, and
//! topological service ordering.

pub mod types;

mod anchor;
mod extends;
mod include;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{ComposeError, Result};
use crate::substitute;
use types::ComposeFile;

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
			if rel_path.is_absolute() {
				return Err(ComposeError::Include(format!(
					"include path must be relative, got absolute path: {rel}"
				)));
			}
			if rel_path
				.components()
				.any(|c| c == std::path::Component::ParentDir)
			{
				return Err(ComposeError::Include(format!(
					"include path must not traverse parent directories: {rel}"
				)));
			}
			let inc_path = dir.join(&rel);
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
	let mut file = deserialize_with_merge(&substituted)?;
	extends::resolve_extends_same_file(&mut file)?;
	Ok(file)
}

/// Parse raw (already-substituted) YAML into a `ComposeFile` without any
/// post-processing.
pub fn parse_str_raw(content: &str) -> Result<ComposeFile> {
	deserialize_with_merge(content)
}

/// Compute a topological start order for all services (Kahn's algorithm).
///
/// Returns service names dependencies-first.
/// Errors on cycles ([`ComposeError::CircularDependency`]) or missing required
/// dependencies ([`ComposeError::ServiceNotFound`]).
pub fn resolve_order(file: &ComposeFile) -> Result<Vec<String>> {
	let services: Vec<&str> = file.services.keys().map(|s| s.as_str()).collect();
	let mut in_degree: HashMap<&str, usize> = services.iter().map(|&s| (s, 0)).collect();
	let mut graph: HashMap<&str, Vec<&str>> = services.iter().map(|&s| (s, vec![])).collect();

	for (name, service) in &file.services {
		for dep in service.depends_on.service_names() {
			if !file.services.contains_key(&dep) {
				if !service.depends_on.required_for(&dep) {
					continue;
				}
				return Err(ComposeError::ServiceNotFound(dep));
			}
			if let Some(neighbors) = graph.get_mut(dep.as_str()) {
				neighbors.push(name.as_str());
			}
			if let Some(deg) = in_degree.get_mut(name.as_str()) {
				*deg += 1;
			}
		}
	}

	let mut queue: std::collections::VecDeque<&str> = in_degree
		.iter()
		.filter(|(_, &deg)| deg == 0)
		.map(|(&s, _)| s)
		.collect();

	let mut order = Vec::new();
	while let Some(node) = queue.pop_front() {
		order.push(node.to_string());
		let neighbors: Vec<&str> = graph.get(node).map_or(&[][..], |v| v.as_slice()).to_vec();
		for neighbor in neighbors {
			if let Some(deg) = in_degree.get_mut(neighbor) {
				*deg -= 1;
				if *deg == 0 {
					queue.push_back(neighbor);
				}
			}
		}
	}

	if order.len() != services.len() {
		return Err(ComposeError::CircularDependency(
			"cycle detected in depends_on".into(),
		));
	}

	Ok(order)
}

pub(crate) fn parse_file_inner(path: &Path, dir: &Path) -> Result<ComposeFile> {
	parse_file_inner_with_env(path, dir, &[])
}

pub(crate) fn parse_file_inner_with_env(
	path: &Path,
	dir: &Path,
	extra_env_files: &[String],
) -> Result<ComposeFile> {
	let content = std::fs::read_to_string(path)
		.map_err(|_| ComposeError::FileNotFound(path.display().to_string()))?;
	let vars = if extra_env_files.is_empty() {
		substitute::build_vars(dir)
	} else {
		substitute::build_vars_with_env_files(dir, extra_env_files)
	};
	let substituted = substitute::substitute(&content, &vars)?;
	deserialize_with_merge(&substituted)
}

fn deserialize_with_merge(content: &str) -> Result<ComposeFile> {
	let mut value: serde_yaml::Value = serde_yaml::from_str(content)?;
	apply_merge_keys(&mut value);
	let file: ComposeFile = serde_yaml::from_value(value)?;
	Ok(file)
}

/// Recursively resolve YAML merge keys (`<<: *anchor`) in a `Value` tree.
///
/// serde_yaml_ng does not expose `apply_merge()` — this replaces it.
/// Merge semantics: keys from the anchor fill in only where the child has no value.
fn apply_merge_keys(value: &mut serde_yaml::Value) {
	match value {
		serde_yaml::Value::Mapping(mapping) => {
			for v in mapping.values_mut() {
				apply_merge_keys(v);
			}
			let merge_key = serde_yaml::Value::String("<<".to_string());
			if let Some(merge_val) = mapping.remove(&merge_key) {
				let bases: Vec<serde_yaml::Mapping> = match merge_val {
					serde_yaml::Value::Mapping(m) => vec![m],
					serde_yaml::Value::Sequence(seq) => seq
						.into_iter()
						.filter_map(|v| match v {
							serde_yaml::Value::Mapping(m) => Some(m),
							_ => None,
						})
						.collect(),
					_ => vec![],
				};
				for base in bases {
					for (k, v) in base {
						if !mapping.contains_key(&k) {
							mapping.insert(k, v);
						}
					}
				}
			}
		}
		serde_yaml::Value::Sequence(seq) => {
			for v in seq.iter_mut() {
				apply_merge_keys(v);
			}
		}
		_ => {}
	}
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

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

	// resolve_order

	#[test]
	fn resolve_order_no_deps_arbitrary_order() {
		let yaml = "services:\n  a:\n    image: x\n  b:\n    image: y\n";
		let file = parse_str_raw(yaml).unwrap();
		let order = resolve_order(&file).unwrap();
		assert_eq!(order.len(), 2);
		assert!(order.contains(&"a".to_string()));
		assert!(order.contains(&"b".to_string()));
	}

	#[test]
	fn resolve_order_dep_before_dependent() {
		let yaml = "services:\n  web:\n    image: nginx\n    depends_on: [db]\n  db:\n    image: postgres\n";
		let file = parse_str_raw(yaml).unwrap();
		let order = resolve_order(&file).unwrap();
		let db_pos = order.iter().position(|s| s == "db").unwrap();
		let web_pos = order.iter().position(|s| s == "web").unwrap();
		assert!(db_pos < web_pos, "db must start before web");
	}

	#[test]
	fn resolve_order_cycle_is_error() {
		let yaml = "services:\n  a:\n    image: x\n    depends_on: [b]\n  b:\n    image: y\n    depends_on: [a]\n";
		let file = parse_str_raw(yaml).unwrap();
		assert!(resolve_order(&file).is_err());
	}

	#[test]
	fn resolve_order_missing_required_dep_is_error() {
		let yaml = "services:\n  web:\n    image: nginx\n    depends_on: [db]\n";
		let file = parse_str_raw(yaml).unwrap();
		assert!(resolve_order(&file).is_err());
	}

	// YAML merge keys (<<)

	#[test]
	fn yaml_merge_key_fills_missing_fields() {
		let yaml = "x-defaults: &defaults\n  image: nginx\n  restart: always\nservices:\n  web:\n    <<: *defaults\n    ports: ['80:80']\n";
		let file = parse_str_raw(yaml).unwrap();
		assert_eq!(file.services["web"].image.as_deref(), Some("nginx"));
	}
}
