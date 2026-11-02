//! Service lists derived either from selected containers or the compose model.

use std::collections::BTreeSet;

use super::{format::table_service, ComposeFile, ContainerListEntry};

/// The distinct service names of the selected containers, ready to print one
/// per line. The label is not ours: a container can carry any text in it, so a
/// control character is escaped like every other cell podup prints, and a
/// container with no service label is left out rather than printed as an empty
/// line.
///
/// The names are told apart before they are escaped, because escaping is not
/// one to one: a real newline and a backslash followed by `n` print the same.
/// Two different services stay two lines, even when the lines look alike.
pub(super) fn container_services(containers: &[ContainerListEntry]) -> Vec<String> {
	containers
		.iter()
		.map(table_service)
		.filter(|name| !name.is_empty())
		.collect::<BTreeSet<_>>()
		.iter()
		.map(|name| crate::ui::sanitize_cell(name))
		.collect()
}

pub(super) fn configured_services<'a>(file: &'a ComposeFile, selected: &[String]) -> Vec<&'a str> {
	file.services
		.keys()
		.filter(|name| selected.is_empty() || selected.contains(name))
		.map(String::as_str)
		.collect()
}
