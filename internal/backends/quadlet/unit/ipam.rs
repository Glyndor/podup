//! Render `ipam.config` pools onto Quadlet's `Subnet=`/`Gateway=`/`IPRange=`.

use std::net::{Ipv4Addr, Ipv6Addr};

use crate::compose::types::IpamPool;

use super::Section;

/// Emit the pools as three repeatable keys. Podman pairs the n-th `Gateway=` and
/// `IPRange=` with the n-th `Subnet=`, so a pool that omits one of them while a
/// later pool sets it would shift that value onto the wrong subnet. Pools before
/// the last one that sets the key therefore get the value Podman would pick for
/// them anyway: the first host address as gateway, the whole subnet as range.
pub(super) fn add_pools(net: &mut Section, pools: &[IpamPool]) {
	let last_gateway = pools.iter().rposition(|p| p.gateway.is_some());
	let last_range = pools.iter().rposition(|p| p.ip_range.is_some());
	for (i, pool) in pools.iter().enumerate() {
		if let Some(subnet) = &pool.subnet {
			net.add("Subnet", subnet.clone());
		}
		let gateway = pool.gateway.clone().or_else(|| {
			let needed = last_gateway.is_some_and(|last| i < last);
			let subnet = pool.subnet.as_deref()?;
			needed.then(|| first_host(subnet)).flatten()
		});
		if let Some(gateway) = gateway {
			net.add("Gateway", gateway);
		}
		let range = pool.ip_range.clone().or_else(|| {
			let needed = last_range.is_some_and(|last| i < last);
			let subnet = pool.subnet.as_deref()?;
			needed.then(|| subnet.to_string())
		});
		if let Some(range) = range {
			net.add("IPRange", range);
		}
	}
}

/// First usable address of a CIDR, the gateway Podman assigns by default.
fn first_host(cidr: &str) -> Option<String> {
	let (addr, prefix) = cidr.split_once('/')?;
	let prefix: u32 = prefix.parse().ok()?;
	if let Ok(v4) = addr.parse::<Ipv4Addr>() {
		if prefix > 30 {
			return None;
		}
		let mask = u32::MAX << (32 - prefix.min(32));
		let mask = if prefix == 0 { 0 } else { mask };
		return Some(Ipv4Addr::from((u32::from(v4) & mask) + 1).to_string());
	}
	let v6 = addr.parse::<Ipv6Addr>().ok()?;
	if prefix > 126 {
		return None;
	}
	let mask = if prefix == 0 {
		0
	} else {
		u128::MAX << (128 - prefix)
	};
	Some(Ipv6Addr::from((u128::from(v6) & mask) + 1).to_string())
}
