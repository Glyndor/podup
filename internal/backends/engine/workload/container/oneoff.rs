//! What a `run` one-off does not inherit from its service.

use std::collections::HashMap;

use crate::compose::types::Service;
use crate::libpod::types::container::PerNetworkOptions;

/// The label every container `run` creates carries.
const ONEOFF_LABEL: &str = "podup.oneoff";

/// Whether `service` is the copy a `run` made for a one-off container.
pub(super) fn is_one_off(service: &Service) -> bool {
	service
		.labels
		.to_map()
		.get(ONEOFF_LABEL)
		.map(String::as_str)
		== Some("true")
}

/// A one-off does not answer to the service's network aliases or to its name.
/// `docker compose run` adds them only with `--use-aliases`; without it a lookup
/// of the service reached the maintenance container as well as the real ones.
/// The container stays reachable by its own name and id.
pub(super) fn drop_service_aliases(
	service: &Service,
	networks: &mut HashMap<String, PerNetworkOptions>,
) {
	if !is_one_off(service) {
		return;
	}
	for options in networks.values_mut() {
		options.aliases.clear();
	}
}

#[cfg(test)]
#[path = "oneoff_tests.rs"]
mod tests;
