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

/// Put the variables of every `--env-from-file` file ahead of the `-e`
/// overrides, as `KEY=VALUE` entries, so a later duplicate (a `-e`) wins.
/// Relative paths resolve against the current directory, and a file that is
/// missing or malformed is an error that names it.
pub(super) fn with_env_from_files(
	files: &[String],
	env_overrides: Vec<String>,
) -> crate::error::Result<Vec<String>> {
	if files.is_empty() {
		return Ok(env_overrides);
	}
	let cwd = std::env::current_dir().map_err(|e| {
		crate::error::ComposeError::EnvFile(format!("cannot resolve --env-from-file: {e}"))
	})?;
	let mut list = Vec::new();
	for file in files {
		let vars = crate::env_file::load_env_files(std::slice::from_ref(file), &cwd).map_err(
			|e| match e {
				not_found @ crate::error::ComposeError::FileNotFound(_) => not_found,
				other => {
					crate::error::ComposeError::EnvFile(format!("--env-from-file {file}: {other}"))
				}
			},
		)?;
		let mut pairs: Vec<_> = vars.into_iter().collect();
		pairs.sort();
		list.extend(pairs.into_iter().map(|(k, v)| format!("{k}={v}")));
	}
	list.extend(env_overrides);
	Ok(list)
}

#[cfg(test)]
mod tests;
