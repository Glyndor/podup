//! The networks a container is attached to: declared `networks:`, the
//! `network_mode:` namespace it joins, and the per-network addresses and
//! aliases.

use crate::compose::types::Service;
use crate::quadlet::is_no_warn_set;

use super::super::super::pod::{PodField, PodFields};
use super::super::super::shared_values::network_values;
use super::super::{sorted_label_pairs, unit_stem, ExternalNames, Section};

/// Emit a `Network=` line per declared network the service attaches to, unless
/// the pod carries the attachment instead.
pub(in super::super) fn emit_networks(
	name: &str,
	service: &Service,
	project: &str,
	declared_networks: &[&str],
	external_networks: &ExternalNames<'_>,
	in_pod: bool,
	container: &mut Section,
) {
	// In pod mode the pod joins the shared namespace, so each container
	// drops its own `Network=` lines; the pod unit carries them. The
	// `Network=` line we emit below (for `network_mode: host`/`none`/
	// `container:`/`service:`) is the one exception: those are
	// namespace modes, not network attachments.
	if !in_pod {
		for net in attachment_order(service) {
			// A declared (non-external) network is backed by a generated `.network`
			// unit; an external network is referenced by its existing name directly,
			// since no unit is emitted for it.
			let target = if declared_networks.contains(&net.as_str()) {
				format!("{}.network", unit_stem(project, &net))
			} else {
				// An external network carries the name it really has, which a
				// compose `name:` may have changed.
				external_networks
					.get(net.as_str())
					.copied()
					.unwrap_or(net.as_str())
					.to_string()
			};
			let options = attachment_options(name, service, &net);
			container.add("Network", format!("{target}:{}", options.join(",")));
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

/// The service's networks in the order the `Network=` lines are written.
///
/// Podman connects them in the order of the `--network` flags, and the first
/// one holds `eth0`. Compose `priority` asks for the highest first, which is
/// what `up` sends as `networkOrder` (`network_order`), so a service that sets
/// one is written the same way: priority descending, then by name, the order
/// libpod uses when it is told none. A service with no priority keeps the
/// order its networks are declared in, as it always has.
fn attachment_order(service: &Service) -> Vec<String> {
	let names = service.networks.names();
	let priority = |net: &str| {
		service
			.networks
			.config_for(net)
			.and_then(|c| c.priority)
			.unwrap_or(0)
	};
	if names.iter().any(|net| priority(net) != 0) {
		crate::engine::network_by_priority(names, priority)
	} else {
		names
	}
}

/// The options Podman reads itself in a `--network` option list. A driver
/// option with one of these names would be read as that option, not handed to
/// the network backend, so it is not written.
const RESERVED_OPTIONS: [&str; 5] = ["alias", "ip", "ip6", "mac", "interface_name"];

/// Whether `text` can sit in a `Network=` option list as it is: no separator,
/// no blank, no control character, nothing that systemd or the list could read
/// as more than one thing.
fn is_plain_option_text(text: &str) -> bool {
	text.chars()
		.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '/' | '@' | '+'))
}

/// The options of one network attachment, the way `up` builds them
/// (`build_per_network_options`): the service name as an alias so siblings
/// resolve it, the aliases declared for this network, its static addresses, its
/// interface name and its driver options. Podman reads them after the network
/// name, so each one is scoped to that network instead of to every network the
/// container joins.
///
/// The value is a comma-separated option list, so an alias, an interface name
/// or a driver option that could break it is left out and reported, and an
/// address is written only once it parses. Podman hands every name it does not
/// know to the network backend, which is how a driver option travels.
fn attachment_options(name: &str, service: &Service, net: &str) -> Vec<String> {
	let config = service.networks.config_for(net);
	let declared = config.and_then(|c| c.aliases.clone()).unwrap_or_default();
	let mut options = Vec::new();
	let mut seen = std::collections::HashSet::new();
	for alias in std::iter::once(name.to_string()).chain(declared) {
		if alias.is_empty() || !seen.insert(alias.clone()) {
			continue;
		}
		if alias
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
		{
			options.push(format!("alias={alias}"));
		} else {
			not_exported(
				name,
				net,
				&format!("network alias {alias:?}"),
				"it holds a character the Network= option list cannot carry",
			);
		}
	}
	let address = |value: Option<&String>| {
		value
			.filter(|v| v.parse::<std::net::IpAddr>().is_ok())
			.cloned()
	};
	if let Some(ip) = address(config.and_then(|c| c.ipv4_address.as_ref())) {
		options.push(format!("ip={ip}"));
	}
	if let Some(ip6) = address(config.and_then(|c| c.ipv6_address.as_ref())) {
		options.push(format!("ip6={ip6}"));
	}
	if let Some(interface) = config.and_then(|c| c.interface_name.as_ref()) {
		if !interface.is_empty() && is_plain_option_text(interface) {
			options.push(format!("interface_name={interface}"));
		} else {
			not_exported(
				name,
				net,
				&format!("interface_name {interface:?}"),
				"it is empty or holds a character the Network= option list cannot carry",
			);
		}
	}
	let driver_opts = config
		.map(|c| sorted_label_pairs(c.driver_opts.clone()))
		.unwrap_or_default();
	for (key, value) in driver_opts {
		if RESERVED_OPTIONS.contains(&key.as_str()) {
			not_exported(
				name,
				net,
				&format!("driver option {key:?}"),
				"Podman reads that name as one of its own network options",
			);
		} else if !key.is_empty() && is_plain_option_text(&key) && is_plain_option_text(&value) {
			options.push(format!("{key}={value}"));
		} else {
			not_exported(
				name,
				net,
				&format!("driver option {key:?}"),
				"its name or value holds a character the Network= option list cannot carry",
			);
		}
	}
	options
}

/// Say that a network setting is not written to the unit, unless warnings are off.
fn not_exported(name: &str, net: &str, what: &str, why: &str) {
	if !is_no_warn_set() {
		tracing::warn!("service \"{name}\": {what} on {net:?} is not exported; {why}");
	}
}

/// In a pod, emit a `--network-alias` podman arg per network alias and the first
/// static address declared across the service's networks as `IP=`/`IP6=`, for
/// the pod fields the pod does not carry. Outside a pod each `Network=` line
/// carries them already.
pub(in super::super) fn emit_network_extras(
	service: &Service,
	in_pod: bool,
	pod_fields: &PodFields,
	container: &mut Section,
) {
	if !in_pod {
		return;
	}
	// In a pod `IP=`/`IP6=` are single-valued per container, so the first static
	// address declared across the service's networks wins; a second one is
	// reported by collect_warnings.
	let values = network_values(service);
	if !(in_pod && pod_fields.owns(PodField::NetworkAlias)) {
		for alias in &values.aliases {
			// An empty alias would render as a bare `NetworkAlias=`, which
			// resets the list instead of adding an alias. An alias that is empty
			// once the control characters are stripped, or that has a space at
			// either end, stays on `PodmanArgs=` (see `Section::add_native`).
			if alias.is_empty() {
				continue;
			}
			container.add_native("NetworkAlias", "--network-alias=", alias);
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
