//! Service lists derived either from selected containers or the compose model.

use std::collections::BTreeSet;

use super::{format::table_service, ComposeFile, ContainerListEntry};

pub(super) fn container_services(containers: &[ContainerListEntry]) -> BTreeSet<String> {
	containers.iter().map(table_service).collect()
}

pub(super) fn configured_services<'a>(file: &'a ComposeFile, selected: &[String]) -> Vec<&'a str> {
	file.services
		.keys()
		.filter(|name| selected.is_empty() || selected.contains(name))
		.map(String::as_str)
		.collect()
}
