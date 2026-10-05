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

/// Stable fragment of the warning, matched by the gate in `mod.rs` that shows
/// it only while `ShowClientAddressWarningGuard` is held.
pub(super) const CLIENT_ADDRESS_NEEDLE: &str =
	"so the container sees every client as one internal address";

/// One warning per service whose published ports reach it through a proxy
/// that replaces the client's address.
pub(super) fn ports_hide_client_address(file: &ComposeFile, out: &mut Vec<String>) {
	// An invalid `x-podman-pod` value is reported elsewhere; treat it as off
	// here rather than warning twice about the same key.
	let in_pod = file.podman_pod().unwrap_or(false);
	// In a pod, the whole project shares one network namespace, so the
	// advice is project-wide: either every service agreed on a mode that
	// keeps the source, or the pod is on a bridge (the only way the
	// namespace is created today) and the user can move it to pasta by
	// declaring the same `network_mode: pasta` on every service.
	let pod_advice = in_pod.then(|| pod_advice_for(file)).flatten();
	for (name, service) in &file.services {
		// Every `ports:` entry publishes: one without a host port is bound to a
		// random host port, not merely exposed.
		if service.ports.is_empty() {
			continue;
		}
		let advice = if let Some(advice) = pod_advice {
			advice.to_string()
		} else {
			match proxy_for(service) {
				Some(Proxy::Rootlessport) => {
					"use network_mode: pasta if it needs the client's address".to_string()
				}
				Some(Proxy::Rootlesskit) => {
					"add port_handler=slirp4netns to the slirp4netns options, or use \
					 network_mode: pasta, if it needs the client's address"
						.to_string()
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

/// What the project-wide advice says when `x-podman-pod: true` is set.
/// Returns `None` when the pod's namespace already keeps the client's
/// address (every service agreed on pasta, or on slirp4netns with
/// `port_handler=slirp4netns` as the last handler), so the caller emits
/// no warning for any service.
fn pod_advice_for(file: &ComposeFile) -> Option<&'static str> {
	match agreed_mode(file) {
		// Every service agreed on pasta, the pod is on pasta: silence.
		Some(agreed) if is_pasta(agreed) => None,
		// Every service agreed on slirp4netns; advice depends on the
		// last `port_handler=` value.
		Some(agreed) if is_slirp4netns(agreed) => {
			if slirp4netns_keeps_source(agreed) {
				None
			} else {
				Some(
					"add port_handler=slirp4netns to the slirp4netns options on every \
					 service, or use network_mode: pasta, if it needs the client's address",
				)
			}
		}
		// No service agreed on a mode, or the agreed mode is neither
		// pasta nor slirp4netns: the pod is on a bridge today, and the
		// fix is to declare the same `network_mode: pasta` everywhere.
		_ => {
			Some("set network_mode: pasta on every service of the pod to keep the client's address")
		}
	}
}

/// The `network_mode` every service agrees on, or `None` when services
/// disagree or none declared one. The pod validator refuses the project
/// in those cases, so the warning's role is purely advisory here.
/// The `network_mode` every service declares, or `None` when they differ or
/// none declares one. Order does not matter.
fn agreed_mode(file: &ComposeFile) -> Option<&str> {
	let mut modes = file.services.values().map(|s| s.network_mode.as_deref());
	let first = modes.next()??;
	modes.all(|m| m == Some(first)).then_some(first)
}

fn is_pasta(mode: &str) -> bool {
	mode == "pasta"
		|| mode
			.strip_prefix("pasta")
			.is_some_and(|r| r.starts_with(':'))
}

fn is_slirp4netns(mode: &str) -> bool {
	mode == "slirp4netns"
		|| mode
			.strip_prefix("slirp4netns")
			.is_some_and(|r| r.starts_with(':'))
}

/// `true` when the last `port_handler=` value in the slirp4netns options
/// is `slirp4netns`. Podman keeps the last one given, so the order in the
/// list decides.
fn slirp4netns_keeps_source(mode: &str) -> bool {
	mode.split_once(':').is_some_and(|(_, opts)| {
		opts.split(',')
			.rev()
			.find_map(|o| o.strip_prefix("port_handler="))
			== Some("slirp4netns")
	})
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
		Some(mode) if is_slirp4netns(mode) => {
			(!slirp4netns_keeps_source(mode)).then_some(Proxy::Rootlesskit)
		}
		// pasta keeps the source; host, none, container: and service: either
		// publish nothing of their own or use the host's addresses directly.
		Some(_) => None,
	}
}

#[cfg(test)]
#[path = "client_address_tests.rs"]
mod tests;
