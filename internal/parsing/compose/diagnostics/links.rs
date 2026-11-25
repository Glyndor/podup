//! Warnings for the service keys that tie one container to another by name:
//! `links`, `external_links` and `domainname`. Podman has no field for any of
//! the three, so what each one does under `up` is said here instead of left
//! to be discovered.

use crate::compose::types::{ComposeFile, Service};

use super::ContainerUnits;

/// The networks a service joins, with the project `default` standing in when it
/// names none, since that is the network it will be attached to.
fn joined_networks(service: &Service) -> Vec<String> {
	if service.network_mode.is_some() {
		return Vec::new();
	}
	if service.networks.is_unset() {
		return vec!["default".to_string()];
	}
	service.networks.names()
}

pub(super) fn link_fields(file: &ComposeFile, units: &ContainerUnits<'_>, out: &mut Vec<String>) {
	for (name, def) in units {
		if def.domainname.is_some() {
			out.push(format!(
				"service '{name}': domainname is not honored: Podman has no setting for a \
				 container's domain name"
			));
		}
		if !def.external_links.is_empty() {
			out.push(format!(
				"service '{name}': external_links is not honored: Podman has no link option, and \
				 a container outside the project cannot be given another name"
			));
		}
		for link in &def.links {
			let (target, alias) = link.split_once(':').unwrap_or((link, link));
			if alias == target {
				continue;
			}
			let Some(linked) = file.services.get(target) else {
				out.push(format!(
					"service '{name}': link '{link}' names no service of this file and is ignored"
				));
				continue;
			};
			let mine = joined_networks(def);
			let shared: Vec<String> = joined_networks(linked)
				.into_iter()
				.filter(|net| mine.contains(net))
				.collect();
			if shared.is_empty() {
				out.push(format!(
					"service '{name}': link '{link}' is ignored: '{name}' and '{target}' share no \
					 network, and Podman has no link option"
				));
			} else {
				out.push(format!(
					"service '{name}': link '{link}' gives '{target}' the network alias \
					 '{alias}' on {}, so every container on {} resolves it, not only '{name}'",
					list(&shared),
					if shared.len() == 1 { "it" } else { "them" },
				));
			}
		}
	}
}

fn list(names: &[String]) -> String {
	names
		.iter()
		.map(|n| format!("'{n}'"))
		.collect::<Vec<_>>()
		.join(", ")
}
