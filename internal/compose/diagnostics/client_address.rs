//! Warn when published ports will hide the client's address (#1994).
//!
//! Under rootless Podman, a port published on a bridge network is forwarded
//! by `rootlessport`, which opens its own connection to the container. The
//! container then sees every client as one internal address, which breaks
//! anything that needs the real one: rate limits, access logs, allowlists, a
//! mail server checking SPF. `network_mode: pasta` forwards with pasta, which
//! keeps the source. Measured 2026-10-04 on Podman 5.7.0 (netavark): the
//! project network and `x-podman-pod` both showed `::ffff:10.89.86.2` for
//! every client, `network_mode: pasta` showed the real source.

use super::super::types::{ComposeFile, PortMapping, Service};

/// Stable fragment of the warning, shared with the suppression gate in
/// `mod.rs` so `ps`, `logs`, `port`, `top` and `--no-warn` silence it the same
/// way as the "published on every interface" warning.
pub(super) const CLIENT_ADDRESS_NEEDLE: &str =
	"so the container sees every client as one internal address";

/// One warning per service that publishes a host port through a bridge.
pub(super) fn ports_hide_client_address(file: &ComposeFile, out: &mut Vec<String>) {
	// An invalid `x-podman-pod` value is reported elsewhere; treat it as off
	// here rather than warning twice about the same key.
	let in_pod = file.podman_pod().unwrap_or(false);
	for (name, service) in &file.services {
		if publishes_host_port(service) && (in_pod || on_bridge(service)) {
			out.push(format!(
				"service '{name}': published ports go through rootlessport on a bridge \
				 network under rootless Podman, {CLIENT_ADDRESS_NEEDLE}; use \
				 network_mode: pasta if it needs the client's address"
			));
		}
	}
}

/// True when the service binds at least one port on the host. A short form
/// with no `:` outside an IPv6 bracket (`"80"`) only exposes the port, and a
/// long form without `published` does not bind one.
fn publishes_host_port(service: &Service) -> bool {
	service.ports.iter().any(|port| match port {
		PortMapping::Short(s) => {
			let no_proto = s.split('/').next().unwrap_or(s);
			no_proto.starts_with('[') || no_proto.contains(':')
		}
		PortMapping::Long { published, .. } => published.is_some(),
	})
}

/// True when the service's traffic goes through a bridge: no `network_mode`
/// (the project's networks), or `bridge` with or without options.
fn on_bridge(service: &Service) -> bool {
	match service.network_mode.as_deref() {
		None => true,
		Some(mode) => mode == "bridge" || mode.starts_with("bridge:"),
	}
}

#[cfg(test)]
#[path = "client_address_tests.rs"]
mod tests;
