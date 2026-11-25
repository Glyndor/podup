//! The order a container is connected to its networks in.

use std::collections::HashMap;

use crate::libpod::types::container::PerNetworkOptions;

/// The names of `networks` in the order the service's `priority` values ask
/// for, or `None` when none of them asks for one.
///
/// The Compose Specification connects the network with the highest `priority`
/// first, so it holds `eth0`. libpod sorts the networks by name when it is not
/// told an order (measured on 6.2.0: with no order the interface of the
/// alphabetically first network is `eth0`; with `networkOrder` the first name is),
/// so an order is sent only when some network sets a priority other than the
/// default of 0. Networks of equal priority keep libpod's own order, by name,
/// which leaves a service without priorities exactly as it was.
pub(in crate::backends::engine) fn network_order(
	networks: &HashMap<String, PerNetworkOptions>,
) -> Option<Vec<String>> {
	if !networks
		.values()
		.any(|options| options.priority.is_some_and(|p| p != 0))
	{
		return None;
	}
	Some(by_priority(networks.keys().cloned().collect(), |name| {
		networks[name].priority.unwrap_or(0)
	}))
}

/// `names` with the highest `priority` first and equal ones by name.
///
/// The lists are a service's networks, a handful of names, so each name is
/// inserted at its place instead of calling `sort_by`: the standard library's
/// stable sort is monomorphised for every element type that uses it, and two
/// uses of it here added about 35 KB to the release binary, which sits within
/// a few KB of `bench/budgets/binary-budget-mib`.
pub(crate) fn by_priority(names: Vec<String>, priority: impl Fn(&str) -> u32) -> Vec<String> {
	let mut ordered: Vec<String> = Vec::with_capacity(names.len());
	for name in names {
		let key = priority(&name);
		let at = ordered
			.iter()
			.position(|other| {
				let other_key = priority(other);
				other_key < key || (other_key == key && other.as_str() > name.as_str())
			})
			.unwrap_or(ordered.len());
		ordered.insert(at, name);
	}
	ordered
}

#[cfg(test)]
#[path = "order_tests.rs"]
mod tests;
