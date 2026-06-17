//! Build the `.network` unit for a declared network.

use crate::compose::types::NetworkConfig;

use super::{safe_unit_stem, sorted_label_pairs, QuadletUnit, Section};

pub(crate) fn network_unit(
	name: &str,
	project: &str,
	config: Option<&NetworkConfig>,
) -> QuadletUnit {
	let mut net = Section::new("Network");
	// A custom `name:` overrides Podman's resource name; Quadlet uses the literal
	// value (no prefix) when `NetworkName=` is set explicitly.
	let net_name = config
		.and_then(|c| c.name.clone())
		.unwrap_or_else(|| format!("{project}_{name}"));
	net.add("NetworkName", net_name);
	if let Some(cfg) = config {
		if let Some(driver) = &cfg.driver {
			net.add("Driver", driver.clone());
		}
		if cfg.internal == Some(true) {
			net.add("Internal", "true".to_string());
		}
		if cfg.enable_ipv6 == Some(true) {
			net.add("IPv6", "true".to_string());
		}
		if let Some(ipam) = &cfg.ipam {
			if let Some(ipam_driver) = &ipam.driver {
				net.add("IPAMDriver", ipam_driver.clone());
			}
			// Each `ipam.config` pool maps to a Subnet=/Gateway=/IPRange= triple;
			// all three keys are repeatable and correlated positionally by Podman.
			for pool in &ipam.config {
				if let Some(subnet) = &pool.subnet {
					net.add("Subnet", subnet.clone());
				}
				if let Some(gateway) = &pool.gateway {
					net.add("Gateway", gateway.clone());
				}
				if let Some(range) = &pool.ip_range {
					net.add("IPRange", range.clone());
				}
			}
		}
		// Each driver option becomes its own Options= line: Quadlet maps one
		// Options= to one `podman network create --opt key=value`, so a
		// comma-joined value would be passed as a single malformed option.
		for (key, val) in sorted_label_pairs(cfg.driver_opts.clone()) {
			net.add("Options", format!("{key}={val}"));
		}
		for (key, val) in sorted_label_pairs(cfg.labels.to_map()) {
			net.add("Label", format!("{key}={val}"));
		}
	}
	let mut contents = net.render();
	contents.push_str("\n[Install]\nWantedBy=default.target\n");
	QuadletUnit {
		filename: format!("{}.network", safe_unit_stem(name)),
		contents,
	}
}
