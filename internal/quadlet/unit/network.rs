//! Build the `.network` unit for a declared network.

use crate::compose::types::NetworkConfig;

use super::{owner_marker, sorted_label_pairs, unit_stem, QuadletUnit, Section};

/// Build the `.network` unit for one declared network. Emits a single `[Network]`
/// section (NetworkName, then driver/internal/IPv6/IPAM/options/labels), always
/// appending the `podup.project` ownership label. No `[Install]` section is
/// written: `.network` units are pulled in as dependencies of the `.container`
/// units that reference them.
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
	// Ownership label, mirroring the live engine: tag every generated network with
	// its project so it is traceable/removable by label like a running one.
	net.add("Label", format!("podup.project={project}"));
	// No [Install] section: the spec defines none for `.network` units, which are
	// pulled in automatically as dependencies of the `.container` units that use
	// them. Only `.container` units carry [Install].
	//
	// The unforgeable ownership marker comes first, as its own comment line;
	// see `owner_marker` for why it must stay separate from the `Label=` line.
	let mut contents = owner_marker(project);
	contents.push_str(&net.render());
	QuadletUnit {
		filename: format!("{}.network", unit_stem(project, name)),
		contents,
	}
}
