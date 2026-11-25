//! The aliases other services' `links:` entries give a service.

use crate::compose::types::{ComposeFile, Service};

/// The aliases `target` answers to on `network` because another service
/// links to it as `target:alias`.
///
/// Podman has no `links` field and no per-container link scope, so the alias
/// is a network alias of the target on a network the linking service also
/// joins. A bare `target` link adds nothing: every service already answers to
/// its own name. Services that name no network join none and are skipped, as
/// are aliases equal to the target's name. The order follows the file, so the
/// result is the same on every run.
pub(crate) fn link_aliases(file: &ComposeFile, target: &str, network: &str) -> Vec<String> {
	link_aliases_among(
		file.services
			.values()
			.chain(file.jobs.values().map(|job| &job.service)),
		target,
		network,
	)
}

/// [`link_aliases`] for the services `linkers` names, for a caller that does not
/// hold the whole file (the Quadlet export holds the services it writes).
pub(crate) fn link_aliases_among<'a>(
	linkers: impl Iterator<Item = &'a Service>,
	target: &str,
	network: &str,
) -> Vec<String> {
	let mut aliases: Vec<String> = Vec::new();
	for service in linkers {
		if service.network_mode.is_some() || !service.networks.names().iter().any(|n| n == network)
		{
			continue;
		}
		for link in &service.links {
			let Some((linked, alias)) = link.split_once(':') else {
				continue;
			};
			if linked == target
				&& alias != target
				&& !alias.is_empty()
				&& !aliases.contains(&alias.to_string())
			{
				aliases.push(alias.to_string());
			}
		}
	}
	aliases
}

#[cfg(test)]
#[path = "link_aliases_tests.rs"]
mod tests;
