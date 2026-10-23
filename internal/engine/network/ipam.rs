//! Translation of compose IPAM pools into the libpod shapes.

use std::collections::HashMap;

use crate::compose::types::IpamConfig;
use crate::libpod::types::network::{LeaseRange, Subnet};

pub(super) fn build_subnets(ipam: &IpamConfig) -> Vec<Subnet> {
	// aux_addresses are reported by the parse-time diagnostics pass (they are not
	// supported by Podman), so the drop is surfaced there rather than logged here.
	ipam.config
		.iter()
		.map(|pool| Subnet {
			subnet: pool.subnet.clone(),
			gateway: pool.gateway.clone(),
			lease_range: pool.ip_range.as_deref().and_then(lease_range_from_cidr),
		})
		.collect()
}

/// Translate `ipam.driver` and `ipam.options` into Podman's `ipam_options` map.
pub(super) fn build_ipam_options(ipam: &IpamConfig) -> HashMap<String, String> {
	let mut opts = ipam.options.clone();
	if let Some(driver) = &ipam.driver {
		opts.insert("driver".to_string(), driver.clone());
	}
	opts
}

/// Convert a compose `ip_range` CIDR into a Podman lease range (the usable
/// host range of the CIDR). Returns `None` for an unparseable CIDR.
fn lease_range_from_cidr(cidr: &str) -> Option<LeaseRange> {
	use std::net::{Ipv4Addr, Ipv6Addr};

	let (addr, prefix) = cidr.split_once('/')?;
	let prefix: u8 = prefix.parse().ok()?;

	if let Ok(v4) = addr.parse::<Ipv4Addr>() {
		if prefix > 32 {
			return None;
		}
		let mask = if prefix == 0 {
			0
		} else {
			u32::MAX << (32 - prefix)
		};
		let base = u32::from(v4) & mask;
		let last = base | !mask;
		// Reserve network and broadcast addresses for non-point-to-point ranges.
		let (start, end) = if prefix >= 31 {
			(base, last)
		} else {
			(base + 1, last - 1)
		};
		return Some(LeaseRange {
			start_ip: Some(Ipv4Addr::from(start).to_string()),
			end_ip: Some(Ipv4Addr::from(end).to_string()),
		});
	}

	if let Ok(v6) = addr.parse::<Ipv6Addr>() {
		if prefix > 128 {
			return None;
		}
		let mask = if prefix == 0 {
			0
		} else {
			u128::MAX << (128 - prefix)
		};
		let base = u128::from(v6) & mask;
		let last = base | !mask;
		return Some(LeaseRange {
			start_ip: Some(Ipv6Addr::from(base).to_string()),
			end_ip: Some(Ipv6Addr::from(last).to_string()),
		});
	}

	None
}

#[cfg(test)]
#[path = "ipam_tests.rs"]
mod tests;
