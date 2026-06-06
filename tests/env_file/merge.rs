use podup::env_file::merge_env;
use std::collections::HashMap;

#[test]
fn service_env_wins_over_env_file() {
	let mut service_env = HashMap::new();
	service_env.insert("KEY".to_string(), Some("from-service".to_string()));

	let mut env_file_vars = HashMap::new();
	env_file_vars.insert("KEY".to_string(), "from-file".to_string());
	env_file_vars.insert("EXTRA".to_string(), "extra".to_string());

	let merged = merge_env(service_env, env_file_vars);
	let map: HashMap<_, _> = merged
		.iter()
		.filter_map(|s| {
			let mut it = s.splitn(2, '=');
			Some((it.next()?.to_string(), it.next()?.to_string()))
		})
		.collect();

	assert_eq!(map["KEY"], "from-service");
	assert_eq!(map["EXTRA"], "extra");
}

#[test]
fn env_file_only_key_included() {
	let service_env = HashMap::new();
	let mut env_file_vars = HashMap::new();
	env_file_vars.insert("FROM_FILE".to_string(), "yes".to_string());

	let merged = merge_env(service_env, env_file_vars);
	assert!(merged.iter().any(|s| s.starts_with("FROM_FILE=")));
}
