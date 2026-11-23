use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::compose::types::Service;
use crate::env_file;
use crate::error::Result;

/// The environment a service's container actually runs with: the service's own
/// `environment:` entries, the values its `env_file:` files carry, and the
/// project environment's value for every bare passthrough key.
///
/// One resolver, shared by the container payload and the config hash, so the
/// two cannot describe different snapshots of the same environment. A `BTreeMap`
/// because the hash folds the entries in and a hash map's iteration order
/// would flap the hash on every parse.
pub(crate) fn effective_env(
	service: &Service,
	base_dir: &Path,
	env_files: &[String],
) -> Result<BTreeMap<String, String>> {
	// The project environment a bare `KEY` reads from: the process, plus `.env`
	// or the global `--env-file` files that replace it.
	let project_vars = crate::substitute::build_vars_with_env_files_strict(base_dir, env_files)?;
	let mut environment = service.environment.clone();
	environment.resolve_bare(&project_vars);

	let entries = service.env_file.to_entries();
	let env_file_vars = if !entries.is_empty() {
		env_file::load_env_file_entries_for(&entries, base_dir, &environment.to_map())?
	} else {
		HashMap::new()
	};
	let mut env = BTreeMap::new();
	// A bare `KEY` the project environment does not define stays undefined, as
	// in docker compose: it is dropped, not set empty.
	for entry in env_file::merge_env(environment.to_map(), env_file_vars) {
		if let Some((key, value)) = entry.split_once('=') {
			env.insert(key.to_string(), value.to_string());
		}
	}
	Ok(env)
}

/// The variables the service's `environment:` names without giving them a
/// value, and that no source resolved: a bare `KEY` the project environment
/// does not define, or a null in a mapping.
///
/// The Compose Specification says such a variable "is unset and is removed
/// from the service container environment". Left out of the environment it
/// would only be absent from what compose sets, and the image's own value of
/// the same name would stay. Podman takes the names to remove in `unsetenv`,
/// which it applies to the image's environment before the service's own, so
/// these are sent there. Sorted, because the request is built from a hash map.
pub(crate) fn unresolved_env_keys(service: &Service, env: &HashMap<String, String>) -> Vec<String> {
	let mut keys: Vec<String> = service
		.environment
		.to_map()
		.into_keys()
		.filter(|key| !env.contains_key(key))
		.collect();
	keys.sort();
	keys
}
