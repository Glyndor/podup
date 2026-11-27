//! The env file that carries the values of a service's `environment:`.
//!
//! Quadlet turns `Environment=KEY=VALUE` into `--env KEY=VALUE` on the
//! `podman run` line, which anyone who can list processes can read. The values
//! go to a file next to the unit instead, which the container unit names with
//! `EnvironmentFile=`; only its path reaches the command line. The file is
//! written like a unit, readable by its owner alone.

use crate::compose::types::Service;

use super::super::{owner_marker, sorted_pairs, unit_stem, Section};
use crate::quadlet::{QuadletUnit, ENV_FILE_SUFFIX};

/// The longest line `podman --env-file` reads, without its line feed (measured
/// on Podman 6.2.1: 65535 bytes are read, a longer line fails the run).
const MAX_LINE: usize = 65_535;

/// Whether `podman --env-file` gives `KEY=VALUE` back as written. It reads the
/// file line by line without quoting or escapes, so a line break cannot be
/// carried, a carriage return before the line feed is dropped, a line that
/// starts with `#` is a comment and the blank space that leads a line is
/// dropped. Other control characters are kept out too, so the file holds
/// nothing a terminal would act on.
pub(crate) fn fits_env_file(key: &str, value: &str) -> bool {
	!key.is_empty()
		&& !key.starts_with(|c: char| c == '#' || c.is_whitespace())
		&& !key.chars().any(char::is_control)
		&& !value.chars().any(|c| c.is_control() && c != '\t')
		&& key.len() + 1 + value.len() <= MAX_LINE
}

/// The `KEY=VALUE` lines of the env file of `service`, sorted by name.
fn lines(service: &Service) -> Vec<String> {
	sorted_pairs(service.environment.to_map())
		.into_iter()
		.filter_map(|(key, value)| {
			let value = value?;
			fits_env_file(&key, &value).then(|| format!("{key}={value}"))
		})
		.collect()
}

fn file_name(project: &str, name: &str) -> String {
	format!("{}{ENV_FILE_SUFFIX}", unit_stem(project, name))
}

/// Name the env file in the container unit. It goes after the service's own
/// `env_file:` entries: Podman lets the last file that names a variable win,
/// and `environment:` wins over `env_file:`. Quadlet reads a relative path
/// against the directory of the unit.
pub(in super::super) fn emit_environment_file(
	project: &str,
	name: &str,
	service: &Service,
	container: &mut Section,
) {
	if !lines(service).is_empty() {
		container.add("EnvironmentFile", file_name(project, name));
	}
}

/// The env file of `service`, or `None` when it has no value to carry. Its first
/// line is the ownership marker of the units, which Podman reads as a comment.
pub(crate) fn environment_file(
	project: &str,
	name: &str,
	service: &Service,
) -> Option<QuadletUnit> {
	let lines = lines(service);
	if lines.is_empty() {
		return None;
	}
	let mut contents = owner_marker(project);
	for line in lines {
		contents.push_str(&line);
		contents.push('\n');
	}
	Some(QuadletUnit {
		filename: file_name(project, name),
		contents,
	})
}
