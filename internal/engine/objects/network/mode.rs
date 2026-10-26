//! Resolution of a service's networking into a netns and its mode options.

use std::collections::HashMap;

use crate::compose::types::{ComposeFile, Service};
use crate::libpod::types::container::{Namespace, PerNetworkOptions};

use super::names::resolve_network_name;
use super::per_network_options::build_per_network_options;

/// Network modes whose options the Podman CLI accepts after a colon
/// (`pasta:-T,15432`). libpod takes the mode and its options separately, so
/// the whole string must not reach it as a mode name (#1994).
const MODES_WITH_OPTIONS: [&str; 2] = ["pasta", "slirp4netns"];

/// Split `pasta:<opts>` or `slirp4netns:<opts>` into the bare mode and its
/// comma-separated options; any other mode is returned unchanged with no
/// options. Empty pieces are dropped, so `pasta:` yields no options.
pub(in crate::engine) fn split_network_mode_options(
	mode: &str,
) -> (&str, Option<(&str, Vec<String>)>) {
	for bare in MODES_WITH_OPTIONS {
		if let Some(rest) = mode.strip_prefix(bare).and_then(|r| r.strip_prefix(':')) {
			let opts: Vec<String> = rest
				.split(',')
				.filter(|o| !o.is_empty())
				.map(str::to_string)
				.collect();
			return (bare, (!opts.is_empty()).then_some((bare, opts)));
		}
	}
	(mode, None)
}

/// The `network_options` map for a service's `network_mode`, empty unless the
/// mode carries options (see [`split_network_mode_options`]).
pub(in crate::engine) fn network_mode_options(service: &Service) -> HashMap<String, Vec<String>> {
	service
		.network_mode
		.as_deref()
		.and_then(|mode| split_network_mode_options(mode).1)
		.map(|(bare, opts)| HashMap::from([(bare.to_string(), opts)]))
		.unwrap_or_default()
}

/// Resolve a service's networking into a netns `Namespace` and per-network
/// options. An explicit `network_mode` wins and yields a namespace with no
/// per-network options (`container:`/`service:` reuse another container's netns,
/// the service form resolved to its target container name). Otherwise each
/// declared network is mapped to its options and `bridge` netns is used (libpod
/// requires `netns=bridge` when explicit networks are attached).
pub(in crate::engine) fn resolve_network_mode(
	service_name: &str,
	service: &Service,
	file: &ComposeFile,
	project: &str,
) -> (Option<Namespace>, HashMap<String, PerNetworkOptions>) {
	if let Some(mode) = &service.network_mode {
		let ns = if let Some(id) = mode.strip_prefix("container:") {
			Namespace::container(id)
		} else if let Some(svc_name) = mode.strip_prefix("service:") {
			let cname = file
				.services
				.get(svc_name)
				.map(|s| resolve_target_container_name(svc_name, s, project))
				.unwrap_or_else(|| svc_name.to_string());
			Namespace::container(cname)
		} else {
			if mode == "bridge" {
				// docker-compose attaches to Docker's shared default `bridge`;
				// Podman reads `--network bridge` as a fresh isolated bridge netns,
				// so project siblings are unreachable. Warn here (at container
				// create) so every path (CLI, library, embedded) surfaces it, not
				// only the parse-time diagnostics.
				tracing::warn!(
					"service '{service_name}': network_mode 'bridge' attaches to a fresh \
					 isolated bridge under Podman, not Docker's shared default bridge, so \
					 project siblings are unreachable; declare a shared `networks:` entry instead"
				);
			}
			Namespace::new(split_network_mode_options(mode).0)
		};
		return (Some(ns), HashMap::new());
	}

	let network_names = service.networks.names();
	let networks: HashMap<String, PerNetworkOptions> = network_names
		.iter()
		.map(|net_name| {
			let full = resolve_network_name(net_name, file, project);
			let opts = build_per_network_options(
				service_name,
				service.networks.config_for(net_name),
				service.mac_address.as_deref(),
			);
			(full, opts)
		})
		.collect();

	// Podman libpod requires netns=bridge when explicit networks are used.
	let netns = (!networks.is_empty()).then(|| Namespace::new("bridge"));

	(netns, networks)
}

/// Resolve the container name that a `network_mode: service:<name>` reference
/// should attach to.
///
/// An explicit `container_name` is honoured verbatim. Otherwise the auto-generated
/// replicas are always index-suffixed `{project}-{svc}-1..N` (there is no
/// base-named container at any replica count), so we point at replica `-1`,
/// matching docker-compose, which attaches `network_mode: service:` references to
/// the first replica.
fn resolve_target_container_name(svc_name: &str, service: &Service, project: &str) -> String {
	if let Some(name) = &service.container_name {
		return name.clone();
	}
	format!("{project}-{svc_name}-1")
}

#[cfg(test)]
#[path = "mode_tests.rs"]
mod tests;
