//! The options services give the networks of an `x-podman-pod` project.
//!
//! The members of a pod share one network namespace, so a network is attached
//! once, to the pod, and not once per service. Podman applies an interface name
//! and the driver options of an attachment when the pod is created (measured on
//! Podman 6.2.0: `podman pod create --network net:interface_name=eth7` gives the
//! members an `eth7`). A service's own `interface_name` and `driver_opts` for a
//! network are therefore carried to the pod when the services that set them
//! agree, and reported when they do not.
//!
//! `priority` orders the attachments of one container. The pod's networks are
//! sent as a set, so it has nothing to order and is reported.

use std::collections::HashMap;

use sha2::{Digest, Sha256};

use crate::compose::types::ComposeFile;
use crate::libpod::types::container::PerNetworkOptions;

/// What the services ask of the pod's networks.
#[derive(Debug, Default)]
pub(crate) struct PodNetworkOptions {
	/// Per network, by the name the pod attaches it under: the interface name
	/// and driver options every service that sets them agrees on.
	pub(crate) applied: Vec<(String, AppliedOptions)>,
	/// What could not be applied, one line each.
	pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct AppliedOptions {
	pub(crate) interface_name: Option<String>,
	pub(crate) driver_opts: HashMap<String, String>,
}

impl AppliedOptions {
	fn is_empty(&self) -> bool {
		self.interface_name.is_none() && self.driver_opts.is_empty()
	}
}

impl PodNetworkOptions {
	/// The options applied to the network the pod attaches under `name`.
	#[cfg(test)]
	pub(crate) fn get(&self, name: &str) -> Option<&AppliedOptions> {
		self.applied
			.iter()
			.find(|(net, _)| net == name)
			.map(|(_, options)| options)
	}
}

/// Read the pod's network options out of the services. `resolve` turns a
/// compose network key into the name the pod attaches it under.
pub(crate) fn pod_network_options(
	file: &ComposeFile,
	resolve: &dyn Fn(&str) -> String,
) -> PodNetworkOptions {
	let mut out = PodNetworkOptions::default();
	for key in file.networks.keys() {
		let mut interface: Vec<(&str, &str)> = Vec::new();
		let mut driver_opts: Vec<(&str, &HashMap<String, String>)> = Vec::new();
		for (service, def) in &file.services {
			let Some(config) = def.networks.config_for(key) else {
				continue;
			};
			if let Some(name) = config.interface_name.as_deref() {
				interface.push((service, name));
			}
			if !config.driver_opts.is_empty() {
				driver_opts.push((service, &config.driver_opts));
			}
			if config.priority.is_some_and(|p| p != 0) {
				out.warnings.push(format!(
					"x-podman-pod: service \"{service}\": networks.{key}.priority orders the networks of one \
					 container and is not applied inside the pod; its networks are attached as a set"
				));
			}
		}
		let mut applied = AppliedOptions::default();
		let mut disagree = |field: &str, who: String| {
			out.warnings.push(format!(
				"x-podman-pod: networks.{key}.{field} differs between {who}; the pod has one \
				 attachment, so none of them is applied"
			));
		};
		match differing(&interface) {
			Some(who) => disagree("interface_name", who),
			None => applied.interface_name = interface.first().map(|(_, name)| name.to_string()),
		}
		match differing(&driver_opts) {
			Some(who) => disagree("driver_opts", who),
			None => {
				applied.driver_opts = driver_opts
					.first()
					.map(|(_, opts)| (*opts).clone())
					.unwrap_or_default();
			}
		}
		if !applied.is_empty() {
			out.applied.push((resolve(key), applied));
		}
	}
	out
}

/// The names of the services whose values differ, or `None` when they all agree
/// (or there are none).
fn differing<T: PartialEq>(entries: &[(&str, T)]) -> Option<String> {
	let (_, first) = entries.first()?;
	if entries.iter().all(|(_, value)| value == first) {
		return None;
	}
	let who: Vec<&str> = entries.iter().map(|(service, _)| *service).collect();
	Some(who.join(", "))
}

/// Fold the options into the hash of the pod. Apart from the hash itself so that
/// the code is written once, whoever hashes.
#[inline(never)]
pub(crate) fn hash_options(hasher: &mut Sha256, file: &ComposeFile) {
	let applied = pod_network_options(file, &str::to_string).applied;
	let mut options: Vec<String> = applied
		.iter()
		.map(|(net, o)| {
			// The driver options are a map with no order: each pair is hashed and
			// the digests are combined with XOR, which does not depend on the order.
			let mut pairs = [0u8; 32];
			for (key, value) in &o.driver_opts {
				let digest = Sha256::digest(format!("{key}\u{1}{value}"));
				pairs
					.iter_mut()
					.zip(digest)
					.for_each(|(acc, byte)| *acc ^= byte);
			}
			format!(
				"{net}\u{1}{}\u{1}{}",
				o.interface_name.as_deref().unwrap_or(""),
				pairs.iter().map(|b| format!("{b:02x}")).collect::<String>()
			)
		})
		.collect();
	options.sort();
	for line in options {
		hasher.update(b"network_options");
		hasher.update(line.as_bytes());
	}
}

/// Put the applied options on the pod's attachments.
pub(crate) fn apply(
	networks: &mut HashMap<String, PerNetworkOptions>,
	options: &PodNetworkOptions,
) {
	for (name, applied) in &options.applied {
		let Some(slot) = networks.get_mut(name) else {
			continue;
		};
		slot.interface_name.clone_from(&applied.interface_name);
		if !applied.driver_opts.is_empty() {
			slot.driver_opts = Some(applied.driver_opts.clone());
		}
	}
}

#[cfg(test)]
#[path = "network_options_tests.rs"]
mod tests;
