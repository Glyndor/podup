use std::collections::HashMap;

#[cfg(test)]
use super::init_service;

/// Layer the three `run` environment sources into the final `KEY=VALUE` / `KEY`
/// list by precedence (`--env-file` < service `environment:` < `-e`), matching
/// `docker compose run --env-file`. `-e` overrides are appended last so a later
/// duplicate wins downstream, mirroring the previous `-e`-only handling.
pub(super) fn merge_run_environment(
	env_file_vars: HashMap<String, String>,
	service_env: HashMap<String, Option<String>>,
	env_overrides: Vec<String>,
) -> Vec<String> {
	// `--env-file` is the base layer; the service's `environment:` overrides it.
	let mut map: HashMap<String, Option<String>> = env_file_vars
		.into_iter()
		.map(|(k, v)| (k, Some(v)))
		.collect();
	for (k, v) in service_env {
		map.insert(k, v);
	}
	let mut env_list: Vec<String> = map
		.into_iter()
		.map(|(k, v)| v.map_or_else(|| k.clone(), |v| format!("{k}={v}")))
		.collect();
	// `-e` overrides win over everything else.
	env_list.extend(env_overrides);
	env_list
}

#[cfg(test)]
mod tests;
