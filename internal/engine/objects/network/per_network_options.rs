//! The options one service carries into one network it is attached to.

use crate::compose::types::ServiceNetworkConfig;
use crate::libpod::types::container::PerNetworkOptions;

/// Build the `PerNetworkOptions` for one service's attachment to a single
/// network: aliases, static IPv4/IPv6 and link-local IPs, MAC (per-network, else
/// `fallback_mac`), driver options (with `priority` folded in), and interface
/// name. The service name is always prepended as an alias unless already present,
/// so siblings resolve the service by name (compose DNS contract).
pub(in crate::engine) fn build_per_network_options(
	service_name: &str,
	cfg: Option<&ServiceNetworkConfig>,
	fallback_mac: Option<&str>,
) -> PerNetworkOptions {
	let mut opts = PerNetworkOptions::default();

	if let Some(c) = cfg {
		opts.aliases = c.aliases.clone().unwrap_or_default();
		if let Some(ipv4) = &c.ipv4_address {
			opts.static_ips.push(ipv4.clone());
		}
		if let Some(ipv6) = &c.ipv6_address {
			opts.static_ips.push(ipv6.clone());
		}
		if !c.link_local_ips.is_empty() {
			opts.static_ips.extend(c.link_local_ips.clone());
		}
		let mac = c.mac_address.as_deref().or(fallback_mac);
		if let Some(m) = mac {
			opts.static_mac = Some(m.to_string());
		}
		// Forward per-attachment driver options. `priority` is surfaced by Compose
		// as a dedicated field but Podman consumes it as a driver option, so fold
		// it in alongside any explicit `driver_opts`.
		let mut driver_opts = c.driver_opts.clone();
		if let Some(prio) = c.priority {
			driver_opts.insert("priority".to_string(), prio.to_string());
		}
		if !driver_opts.is_empty() {
			opts.driver_opts = Some(driver_opts);
		}
		if let Some(iface) = &c.interface_name {
			opts.interface_name = Some(iface.clone());
		}
		// `gw_priority` has no Podman equivalent and is dropped. The user-facing
		// notice is emitted once, at parse time, by the compose diagnostics (see
		// internal/compose/diagnostics/ignored_fields.rs); re-emitting it here on
		// every engine build would double-warn, so no engine-time log is needed.
	} else if let Some(mac) = fallback_mac {
		opts.static_mac = Some(mac.to_string());
	}

	// A service is reachable by its service name on every network it joins
	// (compose-spec DNS contract). Register the service name as a network alias
	// unless the compose file already lists it, so siblings can resolve it by
	// name and not only by the container name or the auto-generated id alias.
	if !service_name.is_empty() && !opts.aliases.iter().any(|a| a == service_name) {
		opts.aliases.insert(0, service_name.to_string());
	}

	opts
}

#[cfg(test)]
#[path = "per_network_options_tests.rs"]
mod tests;
