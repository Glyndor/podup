//! The networks a container is attached to: declared `networks:`, the
//! `network_mode:` namespace it joins, and the per-network addresses and
//! aliases.

use crate::compose::types::Service;
use crate::quadlet::is_no_warn_set;

use super::super::super::pod_fields::{PodField, PodFields};
use super::super::super::shared_values::network_values;
use super::super::{quote_podman_arg_value, unit_stem, Section};

/// Emit a `Network=` line per declared network the service attaches to, unless
/// the pod carries the attachment instead.
pub(in super::super) fn emit_networks(
	service: &Service,
	project: &str,
	declared_networks: &[&str],
	in_pod: bool,
	container: &mut Section,
) {
	// In pod mode the pod joins the shared namespace, so each container
	// drops its own `Network=` lines; the pod unit carries them. The
	// `Network=` line we emit below (for `network_mode: host`/`none`/
	// `container:`/`service:`) is the one exception: those are
	// namespace modes, not network attachments.
	if !in_pod {
		for net in service.networks.names() {
			// A declared (non-external) network is backed by a generated `.network`
			// unit; an external network is referenced by its existing name directly,
			// since no unit is emitted for it.
			if declared_networks.contains(&net.as_str()) {
				container.add("Network", format!("{}.network", unit_stem(project, &net)));
			} else {
				container.add("Network", net.clone());
			}
		}
	}
}

/// Emit the `Network=` line for a namespace `network_mode:`, warning on the
/// modes that hand the container another namespace.
pub(in super::super) fn emit_network_mode(
	name: &str,
	project: &str,
	service: &Service,
	in_pod: bool,
	pod_network_mode: Option<&str>,
	container: &mut Section,
) {
	// `network_mode: host`/`none` map to `Network=host`/`Network=none`.
	// `service:X` reuses a *sibling service's* netns, which Quadlet expresses as
	// `Network={X}.container` (the `.container` unit dependency). `container:X`
	// reuses an *existing* container's netns by id/name and maps to podman's
	// `Network=container:X` join form, not a `.container` unit, which would name
	// a non-existent dependency and fail to start. `pasta` and `slirp4netns`,
	// with or without `:options`, pass through as written: `Network=` takes the
	// same value as `podman run --network` (#1994). Other modes (bridge:,
	// custom, …) have no key and are reported by collect_warnings.
	match service.network_mode.as_deref() {
		Some("host") => {
			container.add("Network", "host".to_string());
			// Quadlet emits the unit file with `Network=host` and walks away;
			// there is no engine call to warn on it. The warning is the same
			// one the live `up` path emits, so the operator sees an identical
			// message whichever path they use. `--no-warn` opts the operator
			// out of this one too.
			if !is_no_warn_set() {
				tracing::warn!(
					"service \"{name}\": network_mode: host shares the host's network namespace; \
					 the container sees host network interfaces and any port it binds is a host port"
				);
			}
		}
		Some("none") => container.add("Network", "none".to_string()),
		Some(m) => {
			if let Some(target) = m.strip_prefix("service:") {
				container.add(
					"Network",
					format!("{}.container", unit_stem(project, target)),
				);
			} else if is_rootless_user_mode(m) {
				if !in_pod {
					container.add("Network", m.to_string());
				} else if Some(m) != pod_network_mode && !is_no_warn_set() {
					// A pod member joins the pod's namespace, and Podman refuses
					// a second network there, so the mode is not written for it.
					// When the pod itself carries this mode, the per-service
					// declaration is just stating what the pod is already on,
					// and nothing is lost; stay quiet.
					tracing::warn!(
						"service \"{name}\": network_mode {m:?} is ignored inside the \
						 x-podman-pod pod; members use the pod's network namespace"
					);
				}
			} else if let Some(target) = m.strip_prefix("container:") {
				container.add("Network", format!("container:{target}"));
				// Sharing another container's netns collides with the same
				// isolation argument as `host`. Podman does not warn on it at
				// generate time, so the surface lives here. `--no-warn` opts
				// the operator out of this one too.
				if !is_no_warn_set() {
					tracing::warn!(
						"service \"{name}\": network_mode: container:{target} shares another \
						 container's network namespace; both containers see the same network \
						 interfaces and ports"
					);
				}
			}
		}
		None => {}
	}
}

/// Emit a `--network-alias` podman arg per network alias, and the first static
/// address declared across the service's networks as `IP=`/`IP6=`.
pub(in super::super) fn emit_network_extras(
	service: &Service,
	in_pod: bool,
	pod_fields: &PodFields,
	container: &mut Section,
) {
	// `IP=`/`IP6=` are single-valued per container, so the first static address
	// declared across the service's networks wins (Quadlet has no per-network IP
	// scoping); a second one is reported by collect_warnings.
	let values = network_values(service);
	if !(in_pod && pod_fields.owns(PodField::NetworkAlias)) {
		for alias in &values.aliases {
			// An empty alias would render as `--network-alias=`,
			// which podman rejects. The native `NetworkAlias=`
			// key omits empty values, so the podman-args route
			// has to skip them too.
			if alias.is_empty() {
				continue;
			}
			// `NetworkAlias=` was added to Quadlet in Podman 5.2.0.
			// The supported floor is 5.0, so route each alias
			// through `PodmanArgs=` as `--network-alias`, like
			// the other escape-hatch flags. Quoted per #1734.
			container.add(
				"PodmanArgs",
				format!("--network-alias={}", quote_podman_arg_value(alias)),
			);
		}
	}
	if !(in_pod && pod_fields.owns(PodField::Ip)) {
		if let Some(ip) = values.ipv4 {
			container.add("IP", ip);
		}
	}
	if !(in_pod && pod_fields.owns(PodField::Ip6)) {
		if let Some(ip6) = values.ipv6 {
			container.add("IP6", ip6);
		}
	}
}

/// `pasta` or `slirp4netns`, bare or with `:options`: the user-mode network
/// modes `Network=` accepts verbatim.
pub(crate) fn is_rootless_user_mode(mode: &str) -> bool {
	["pasta", "slirp4netns"]
		.iter()
		.any(|m| mode == *m || mode.strip_prefix(m).is_some_and(|r| r.starts_with(':')))
}
