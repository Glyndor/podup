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
use types::{ComposeFile, ServiceNetworks};

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

/// Collect parse-time diagnostics for an already-parsed compose file: warnings
/// about recognized-but-unsupported keys and fields that are accepted but carry
/// no effect on Podman. The CLI prints these automatically; library consumers
/// (e.g. panel-agent) can call this to surface the same warnings, since
/// [`parse_file`] does not emit them itself.
pub fn collect_diagnostics(file: &ComposeFile) -> Vec<String> {
	diagnostics::collect(file)
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
	normalize_default_network(&mut merged);
	for warning in diagnostics::collect(&merged) {
		tracing::warn!("{warning}");
	}
	Ok(merged)
}

/// Synthesize the implicit `default` network, matching docker-compose: any
/// service that declares neither `networks:` nor `network_mode` is attached to
/// a project `default` network. Without this, such services are created with no
/// network namespace at all — they get no IP and cannot resolve each other by
/// name, silently breaking the common no-`networks:`-block compose file.
///
/// The `default` network is created as `{project}_default` (see
/// `resolve_network_name`) unless the file already defines a top-level
/// `networks.default`, whose configuration is then respected. Idempotent.
pub(crate) fn normalize_default_network(file: &mut ComposeFile) {
	let needs_default = file
		.services
		.values()
		.any(|svc| svc.network_mode.is_none() && matches!(svc.networks, ServiceNetworks::Empty));
	if !needs_default {
		return;
	}
	file.networks.entry("default".to_string()).or_insert(None);
	for svc in file.services.values_mut() {
		if svc.network_mode.is_none() && matches!(svc.networks, ServiceNetworks::Empty) {
			svc.networks = ServiceNetworks::List(vec!["default".to_string()]);
		}
	}
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
	fn collect_diagnostics_surfaces_unknown_keys() {
		// The public helper lets library consumers see the same warnings the CLI
		// prints; parse_file itself stays quiet.
		let file =
			parse_str_raw("services:\n  web:\n    image: nginx\n    enviroment:\n      - A=1\n")
				.unwrap();
		let diags = collect_diagnostics(&file);
		assert!(
			diags.iter().any(|d| d.contains("enviroment")),
			"expected an unknown-key diagnostic, got {diags:?}"
		);
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

	// Default-network synthesis (#417)

	#[test]
	fn normalize_attaches_bare_service_to_default_network() {
		let mut file = parse_str("services:\n  web:\n    image: nginx\n").unwrap();
		normalize_default_network(&mut file);
		assert!(file.networks.contains_key("default"));
		assert_eq!(file.services["web"].networks.names(), vec!["default"]);
	}

	#[test]
	fn normalize_leaves_service_with_explicit_networks_untouched() {
		let mut file = parse_str(
			"services:\n  web:\n    image: nginx\n    networks: [front]\nnetworks:\n  front:\n",
		)
		.unwrap();
		normalize_default_network(&mut file);
		assert_eq!(file.services["web"].networks.names(), vec!["front"]);
		// No default network is synthesized when nothing needs it.
		assert!(!file.networks.contains_key("default"));
	}

	#[test]
	fn normalize_skips_service_with_network_mode() {
		let mut file =
			parse_str("services:\n  web:\n    image: nginx\n    network_mode: host\n").unwrap();
		normalize_default_network(&mut file);
		assert!(file.services["web"].networks.names().is_empty());
		assert!(!file.networks.contains_key("default"));
	}

	#[test]
	fn normalize_respects_explicit_default_network_config() {
		let mut file = parse_str(
			"services:\n  web:\n    image: nginx\nnetworks:\n  default:\n    driver: bridge\n",
		)
		.unwrap();
		normalize_default_network(&mut file);
		// The user-defined `default` config is kept, not overwritten with None.
		assert!(file.networks["default"].is_some());
		assert_eq!(file.services["web"].networks.names(), vec!["default"]);
	}
}
