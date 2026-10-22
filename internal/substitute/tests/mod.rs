use super::*;

fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
	pairs
		.iter()
		.map(|(k, v)| (k.to_string(), v.to_string()))
		.collect()
}

mod budgets;
mod dotenv;
mod env_files;
mod expansion;
mod modifiers;
