use super::*;

fn peekable(s: &str) -> std::iter::Peekable<std::str::Chars<'_>> {
	s.chars().peekable()
}

fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
	pairs
		.iter()
		.map(|(k, v)| ((*k).to_string(), (*v).to_string()))
		.collect()
}

mod braced_variables;
mod modifiers;
