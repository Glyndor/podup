use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::compose::types::Service;
use crate::env_file;
use crate::error::Result;

/// The environment a service's container actually runs with: the service's own
/// `environment:` entries, the values its `env_file:` files carry, and the
/// host's value for every bare passthrough key.
///
/// One resolver, shared by the container payload and the config hash, so the
/// two cannot describe different snapshots of the same environment. A `BTreeMap`
/// because the hash folds the entries in and a hash map's iteration order
/// would flap the hash on every parse.
pub(crate) fn effective_env(
	service: &Service,
	base_dir: &Path,
) -> Result<BTreeMap<String, String>> {
	let entries = service.env_file.to_entries();
	let env_file_vars = if !entries.is_empty() {
		env_file::load_env_file_entries(&entries, base_dir)?
	} else {
		HashMap::new()
	};
	let mut env = BTreeMap::new();
	for entry in env_file::merge_env(service.environment.to_map(), env_file_vars) {
		match entry.find('=') {
			Some(idx) => {
				env.insert(entry[..idx].to_string(), entry[idx + 1..].to_string());
			}
			// A bare `KEY` (no `=`) is a passthrough: its value comes from
			// podup's own environment, matching docker-compose. Drop it only
			// when unset.
			None => {
				if let Ok(value) = std::env::var(&entry) {
					env.insert(entry, value);
				}
			}
		}
	}
	Ok(env)
}
