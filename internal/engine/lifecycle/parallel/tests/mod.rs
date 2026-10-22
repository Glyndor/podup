use super::*;
use crate::compose::types::Service;
use crate::error::ComposeError;

mod dispatch;
mod levels;
mod restart_set;

fn levels(input: &[&[&str]]) -> Vec<Vec<String>> {
	input
		.iter()
		.map(|lvl| lvl.iter().map(|s| s.to_string()).collect())
		.collect()
}

fn file_with(services: &[&str]) -> ComposeFile {
	let mut file = ComposeFile::default();
	for &s in services {
		file.services.insert(s.to_string(), Service::default());
	}
	file
}
