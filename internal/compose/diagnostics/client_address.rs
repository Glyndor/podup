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
//! slirp4netns's default `port_handler=rootlesskit` rewrites the source the
//! same way; `port_handler=slirp4netns` keeps it, per `podman-run(1)`.

use super::super::types::{ComposeFile, Service};

/// Stable fragment of the warning, shared with the suppression gate in
/// `mod.rs` so `ps`, `logs`, `port`, `top` and `--no-warn` silence it the same
/// way as the "published on every interface" warning.
pub(super) const CLIENT_ADDRESS_NEEDLE: &str =
	"so the container sees every client as one internal address";

/// One warning per service whose published ports reach it through a proxy
/// that replaces the client's address.
pub(super) fn ports_hide_client_address(file: &ComposeFile, out: &mut Vec<String>) {
	// An invalid `x-podman-pod` value is reported elsewhere; treat it as off
	// here rather than warning twice about the same key.
	let in_pod = file.podman_pod().unwrap_or(false);
	for (name, service) in &file.services {
		// Every `ports:` entry publishes: one without a host port is bound to a
		// random host port, not merely exposed.
		if service.ports.is_empty() {
			continue;
		}
		let advice = if in_pod {
			"a pod cannot use pasta yet, so run a service that needs the client's \
			 address outside the pod with network_mode: pasta"
		} else {
			match proxy_for(service) {
				Some(Proxy::Rootlessport) => {
					"use network_mode: pasta if it needs the client's address"
				}
				Some(Proxy::Rootlesskit) => {
					"add port_handler=slirp4netns to the slirp4netns options, or use \
					 network_mode: pasta, if it needs the client's address"
				}
				None => continue,
			}
		};
		out.push(format!(
			"service '{name}': under rootless Podman its published ports are forwarded \
			 by a proxy, {CLIENT_ADDRESS_NEEDLE}; {advice}"
		));
	}
}

/// The proxy that forwards a service's published ports, when it is one that
/// replaces the client's address.
enum Proxy {
	/// `rootlessport`, for a bridge network.
	Rootlessport,
	/// slirp4netns's default `port_handler=rootlesskit`.
	Rootlesskit,
}

fn proxy_for(service: &Service) -> Option<Proxy> {
	match service.network_mode.as_deref() {
		None => Some(Proxy::Rootlessport),
		Some(mode) if mode == "bridge" || mode.starts_with("bridge:") => Some(Proxy::Rootlessport),
		Some(mode) if mode == "slirp4netns" || mode.starts_with("slirp4netns:") => {
			let keeps_source = mode
				.split_once(':')
				.is_some_and(|(_, opts)| opts.split(',').any(|o| o == "port_handler=slirp4netns"));
			(!keeps_source).then_some(Proxy::Rootlesskit)
		}
		// pasta keeps the source; host, none, container: and service: either
		// publish nothing of their own or use the host's addresses directly.
		Some(_) => None,
	}
}

#[cfg(test)]
#[path = "client_address_tests.rs"]
mod tests;
