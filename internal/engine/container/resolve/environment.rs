use std::collections::HashMap;
use std::path::Path;

use crate::compose::types::Service;
use crate::env_file;
use crate::error::Result;

pub(in crate::engine::container) fn build_env(
	service: &Service,
	base_dir: &Path,
) -> Result<Vec<String>> {
	let entries = service.env_file.to_entries();
	let env_file_vars = if !entries.is_empty() {
		env_file::load_env_file_entries(&entries, base_dir)?
	} else {
		HashMap::new()
	};
	Ok(env_file::merge_env(
		service.environment.to_map(),
		env_file_vars,
	))
}
