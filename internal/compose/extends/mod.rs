//! `extends:` directive — inheritance and field merging between service definitions.
//!
//! Services can extend another service within the same file or from an external
//! compose file referenced by path. Resolution is recursive (chains are supported)
//! and cycle detection uses a visited set to error early.
//!
//! Merge semantics: scalar fields from the child win; collection fields
//! (env vars, labels, vectors) are merged with the child taking precedence on
//! overlapping keys. See [`merge_service`] for full field-by-field rules.

mod merge;

use std::collections::HashSet;
use std::path::Path;

use super::parse_file_inner;
use super::types::ComposeFile;
use crate::error::{ComposeError, Result};

pub(in crate::compose) use merge::merge_service;

const MAX_EXTENDS_DEPTH: usize = 16;

/// Resolve `extends:` only within the same file (no `file:` references).
///
/// Used by [`super::parse_str`] where there is no on-disk path.
pub(super) fn resolve_extends_same_file(file: &mut ComposeFile) -> Result<()> {
	let names: Vec<String> = file.services.keys().cloned().collect();
	for name in names {
		let mut visited: HashSet<String> = HashSet::new();
		resolve_one_extends_in_memory(file, &name, &mut visited, 0)?;
	}
	Ok(())
}

/// Resolve `extends:` for every service in `file`, including chains across
/// other compose files referenced by `extends.file`.
pub(super) fn resolve_all_extends(file: &mut ComposeFile, base_dir: &Path) -> Result<()> {
	let names: Vec<String> = file.services.keys().cloned().collect();
	for name in names {
		let mut visited: HashSet<String> = HashSet::new();
		resolve_one_extends(file, &name, base_dir, &mut visited, 0)?;
	}
	Ok(())
}

fn resolve_one_extends_in_memory(
	file: &mut ComposeFile,
	name: &str,
	visited: &mut HashSet<String>,
	depth: usize,
) -> Result<()> {
	if depth >= MAX_EXTENDS_DEPTH {
		return Err(ComposeError::Extends(format!(
			"extends chain exceeds maximum depth ({MAX_EXTENDS_DEPTH}) at service '{name}'"
		)));
	}
	if !visited.insert(name.to_string()) {
		return Err(ComposeError::Extends(format!("circular extends at {name}")));
	}

	let extends = match file.services.get(name).and_then(|s| s.extends.clone()) {
		Some(e) => e,
		None => return Ok(()),
	};

	if extends.file().is_some() {
		return Err(ComposeError::Extends(format!(
			"service '{name}' uses 'extends.file' but parser was given a string, not a path"
		)));
	}

	let base_name = extends.service().to_string();
	if base_name == name {
		return Err(ComposeError::Extends(format!(
			"service '{name}' extends itself"
		)));
	}

	if file.services.get(&base_name).is_none() {
		return Err(ComposeError::Extends(format!(
			"service '{name}' extends unknown service '{base_name}'"
		)));
	}
	resolve_one_extends_in_memory(file, &base_name, visited, depth + 1)?;

	let base = file
		.services
		.get(&base_name)
		.cloned()
		.ok_or_else(|| ComposeError::Extends(base_name.clone()))?;

	if let Some(svc) = file.services.get_mut(name) {
		let merged = merge_service(base, svc.clone());
		*svc = merged;
		svc.extends = None;
	}

	Ok(())
}

fn resolve_one_extends(
	file: &mut ComposeFile,
	name: &str,
	base_dir: &Path,
	visited: &mut HashSet<String>,
	depth: usize,
) -> Result<()> {
	if depth >= MAX_EXTENDS_DEPTH {
		return Err(ComposeError::Extends(format!(
			"extends chain exceeds maximum depth ({MAX_EXTENDS_DEPTH}) at service '{name}'"
		)));
	}
	if !visited.insert(name.to_string()) {
		return Err(ComposeError::Extends(format!("circular extends at {name}")));
	}

	let extends = match file.services.get(name).and_then(|s| s.extends.clone()) {
		Some(e) => e,
		None => return Ok(()),
	};

	let base_name = extends.service().to_string();

	let base_service = if let Some(file_path) = extends.file() {
		if !is_safe_extends_path(file_path) {
			return Err(ComposeError::Extends(format!(
				"service '{name}' extends.file must be a relative path with no parent traversal: {file_path}"
			)));
		}
		let abs = base_dir.join(file_path);
		let abs = abs.canonicalize().unwrap_or(abs);
		let dir = abs
			.parent()
			.map(|p| p.to_path_buf())
			.unwrap_or_else(|| base_dir.to_path_buf());
		let mut other = parse_file_inner(&abs, &dir)?;
		let mut nested_visited: HashSet<String> = HashSet::new();
		resolve_one_extends(&mut other, &base_name, &dir, &mut nested_visited, depth + 1)?;
		let mut base = other.services.swap_remove(&base_name).ok_or_else(|| {
			ComposeError::Extends(format!(
				"service '{base_name}' not found in {}",
				abs.display()
			))
		})?;
		// The base service's relative paths are relative to the external file's
		// directory; anchor them before merging into the current file's service.
		super::anchor::anchor_service(&mut base, &dir);
		base
	} else {
		if base_name == name {
			return Err(ComposeError::Extends(format!(
				"service '{name}' extends itself"
			)));
		}
		if !file.services.contains_key(&base_name) {
			return Err(ComposeError::Extends(format!(
				"service '{name}' extends unknown service '{base_name}'"
			)));
		}
		resolve_one_extends(file, &base_name, base_dir, visited, depth + 1)?;
		file.services
			.get(&base_name)
			.cloned()
			.ok_or_else(|| ComposeError::Extends(base_name.clone()))?
	};

	if let Some(svc) = file.services.get_mut(name) {
		let merged = merge_service(base_service, svc.clone());
		*svc = merged;
		svc.extends = None;
	}

	Ok(())
}

/// Security check: reject `extends.file` with absolute or parent-traversing paths.
///
/// Exposed for unit testing.
pub(crate) fn is_safe_extends_path(path: &str) -> bool {
	crate::fsutil::is_safe_relative_path(path)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;
	use crate::compose::types::{ComposeFile, EnvVars, Labels, Service};
	use indexmap::IndexMap;

	fn svc(image: &str) -> Service {
		Service {
			image: Some(image.to_string()),
			..Default::default()
		}
	}

	// is_safe_extends_path

	#[test]
	fn safe_path_relative() {
		assert!(is_safe_extends_path("other.yml"));
	}

	#[test]
	fn safe_path_subdirectory() {
		assert!(is_safe_extends_path("bases/db.yml"));
	}

	#[test]
	fn unsafe_path_absolute() {
		assert!(!is_safe_extends_path("/etc/compose.yml"));
	}

	#[test]
	fn unsafe_path_parent_traversal() {
		assert!(!is_safe_extends_path("../secret.yml"));
	}

	#[test]
	fn unsafe_path_nested_traversal() {
		assert!(!is_safe_extends_path("a/../../etc/passwd"));
	}

	// merge_service — scalar fields

	#[test]
	fn merge_override_image_wins() {
		let base = svc("nginx:1.24");
		let over = svc("nginx:1.25");
		let merged = merge_service(base, over);
		assert_eq!(merged.image.as_deref(), Some("nginx:1.25"));
	}

	#[test]
	fn merge_base_image_used_when_override_missing() {
		let base = svc("nginx:1.24");
		let over = Service::default();
		let merged = merge_service(base, over);
		assert_eq!(merged.image.as_deref(), Some("nginx:1.24"));
	}

	// merge_service — env vars (override wins per key)

	#[test]
	fn merge_env_vars_override_wins_on_conflict() {
		let mut base_map = IndexMap::new();
		base_map.insert(
			"PORT".to_string(),
			Some(serde_yaml::Value::String("8080".into())),
		);
		let base = Service {
			environment: EnvVars::Map(base_map),
			..Default::default()
		};

		let mut over_map = IndexMap::new();
		over_map.insert(
			"PORT".to_string(),
			Some(serde_yaml::Value::String("9090".into())),
		);
		let over = Service {
			environment: EnvVars::Map(over_map),
			..Default::default()
		};

		let merged = merge_service(base, over);
		let env = merged.environment.to_map();
		assert_eq!(env.get("PORT").and_then(|v| v.as_deref()), Some("9090"));
	}

	#[test]
	fn merge_env_vars_base_key_preserved_when_not_overridden() {
		let mut base_map = IndexMap::new();
		base_map.insert(
			"BASE_ONLY".to_string(),
			Some(serde_yaml::Value::String("yes".into())),
		);
		let base = Service {
			environment: EnvVars::Map(base_map),
			..Default::default()
		};
		let over = Service::default();
		let merged = merge_service(base, over);
		let env = merged.environment.to_map();
		assert_eq!(env.get("BASE_ONLY").and_then(|v| v.as_deref()), Some("yes"));
	}

	// merge_service — labels (merged, override wins on conflict)

	#[test]
	fn merge_labels_both_preserved() {
		let mut base_im = IndexMap::new();
		base_im.insert("team".to_string(), "infra".to_string());
		let base = Service {
			labels: Labels::Map(base_im),
			..Default::default()
		};

		let mut over_im = IndexMap::new();
		over_im.insert("env".to_string(), "prod".to_string());
		let over = Service {
			labels: Labels::Map(over_im),
			..Default::default()
		};

		let merged = merge_service(base, over);
		let lm = merged.labels.to_map();
		assert_eq!(lm.get("team").map(|s| s.as_str()), Some("infra"));
		assert_eq!(lm.get("env").map(|s| s.as_str()), Some("prod"));
	}

	// resolve_extends_same_file — cycle detection

	#[test]
	fn cycle_detection_returns_error() {
		use crate::compose::types::ExtendsConfig;
		let mut file = ComposeFile::default();
		file.services.insert(
			"a".to_string(),
			Service {
				extends: Some(ExtendsConfig::Service("b".to_string())),
				..Default::default()
			},
		);
		file.services.insert(
			"b".to_string(),
			Service {
				extends: Some(ExtendsConfig::Service("a".to_string())),
				..Default::default()
			},
		);
		assert!(resolve_extends_same_file(&mut file).is_err());
	}

	#[test]
	fn self_extends_returns_error() {
		use crate::compose::types::ExtendsConfig;
		let mut file = ComposeFile::default();
		file.services.insert(
			"web".to_string(),
			Service {
				extends: Some(ExtendsConfig::Service("web".to_string())),
				..Default::default()
			},
		);
		assert!(resolve_extends_same_file(&mut file).is_err());
	}

	#[test]
	fn extends_unknown_service_returns_error() {
		use crate::compose::types::ExtendsConfig;
		let mut file = ComposeFile::default();
		file.services.insert(
			"web".to_string(),
			Service {
				extends: Some(ExtendsConfig::Service("nonexistent".to_string())),
				..Default::default()
			},
		);
		assert!(resolve_extends_same_file(&mut file).is_err());
	}

	#[test]
	fn extends_inherits_image_from_base() {
		use crate::compose::types::ExtendsConfig;
		let mut file = ComposeFile::default();
		file.services.insert("base".to_string(), svc("postgres:16"));
		file.services.insert(
			"db".to_string(),
			Service {
				extends: Some(ExtendsConfig::Service("base".to_string())),
				..Default::default()
			},
		);
		resolve_extends_same_file(&mut file).unwrap();
		assert_eq!(file.services["db"].image.as_deref(), Some("postgres:16"));
		assert!(file.services["db"].extends.is_none());
	}
}
