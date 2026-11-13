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
			// `default` and empty mean the engine's own IPAM; Quadlet rejects them.
			if let Some(ipam_driver) = ipam
				.driver
				.as_ref()
				.filter(|d| !matches!(d.trim(), "" | "default"))
			{
				net.add("IPAMDriver", ipam_driver.clone());
			}
			super::ipam::add_pools(&mut net, &ipam.config);
		}
	}
	// Driver options, with the same default the live engine applies to a bridge
	// network: isolate it from the other bridge networks unless the file said
	// otherwise. Without it a network exported to Quadlet would let a container
	// reach the unpublished ports of a neighbouring network, which `podup up`
	// does not. Each option is its own Options= line: Quadlet maps one Options=
	// to one `podman network create --opt key=value`, so a comma-joined value
	// would be passed as a single malformed option.
	let driver = config
		.and_then(|c| c.driver.clone())
		.unwrap_or_else(|| "bridge".to_string());
	let mut opts = config.map(|c| c.driver_opts.clone()).unwrap_or_default();
	crate::engine::apply_default_isolation(&driver, &mut opts);
	for (key, val) in sorted_label_pairs(opts) {
		net.add("Options", format!("{key}={val}"));
	}
	if let Some(cfg) = config {
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
