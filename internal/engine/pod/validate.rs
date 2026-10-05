//! Pre-flight validation for the `x-podman-pod` extension.
//!
//! `up`/`create` call [`validate_pod_or_refuse`] before any container or
//! pod is created, so a configuration the pod cannot honour is reported
//! up front with a message naming the service and the offending key, not
//! mid-create as an opaque libpod 500.
//!
//! Refusals:
//!
//! - `network_mode` declared on some services but not all, or on no
//!   service, or on every service but in different forms: a pod already
//!   pins every container to one shared namespace, and a per-service
//!   `network_mode` would override that. The exception is a project where
//!   every service agrees on the same `network_mode`, and that mode is
//!   `pasta` or `slirp4netns` (bare or with `:options`): podup creates
//!   the pod on that mode and lets every member join, so each container
//!   keeps the client's address. Any other case is refused with a message
//!   that names the offending service and the disagreement.
//! - `network_mode` agreed on across services, and at least one service
//!   also declares `networks:`. libpod refuses the mix on a pod.
//! - Two services with divergent `networks:` sets: every service has to be
//!   on the same set (or declare none and get the project default). The
//!   pod's `networks` map is built from every declared network, and two
//!   services that disagree would either leave some unreachable or pull
//!   in extras. Per-service sets are checked against the first non-empty
//!   set seen.
//! - Two services publishing the same host port: pod mode hands the
//!   union of every service's `ports:` to the pod, and Podman rejects
//!   duplicate host ports with HTTP 500. Pre-empt it.
//! - Two services publishing the same port on different host IPs: the
//!   union collapses the two into the same Podman entry, and the per-IP
//!   binding is silently lost. Refuse so the user picks one or the other.

use indexmap::IndexSet;

use crate::compose::types::{ComposeFile, Service};
use crate::quadlet::is_rootless_user_mode;

/// Pre-flight check: refuse any compose-file shape the pod cannot honour.
/// `Err` messages name the service and the offending key, so the user can
/// fix the source.
pub(crate) fn validate_pod_or_refuse(file: &ComposeFile) -> Result<(), String> {
	let services: Vec<(&str, &Service)> =
		file.services.iter().map(|(k, v)| (k.as_str(), v)).collect();

	// 1. `network_mode` must agree on every service. The one agreed mode
	//    may be unset (no service declares one), or it may be `pasta` or
	//    `slirp4netns` (bare or with `:options`). Any other case is
	//    refused: a different per-service mode would override the pod's
	//    shared namespace, a partial declaration is a disagreement, and
	//    the engine's spec builder only handles the pasta/slirp4netns case.
	// Either no service declares `network_mode`, or every service declares the
	// same one. Order must not matter: a service without it is a disagreement
	// whether it comes before or after one that has it.
	let declared = services
		.iter()
		.find_map(|(n, s)| s.network_mode.as_deref().map(|m| (*n, m)));
	let agreed_mode: Option<String> = match declared {
		None => None,
		Some((first, agreed)) => {
			if let Some((name, other)) = services
				.iter()
				.map(|(n, s)| (*n, s.network_mode.as_deref()))
				.find(|(_, m)| *m != Some(agreed))
			{
				let shown = other.map_or_else(|| "(unset)".to_string(), |m| format!("{m:?}"));
				return Err(format!(
					"service \"{name}\": network_mode {shown} differs from service \"{first}\" \
					 ({agreed:?}); in x-podman-pod every service must declare the same \
					 network_mode, or none"
				));
			}
			Some(agreed.to_string())
		}
	};
	let first_with_mode = declared.map(|(n, _)| n);
	if let Some(agreed) = &agreed_mode {
		if !is_rootless_user_mode(agreed) {
			let first = first_with_mode.unwrap_or("?");
			return Err(format!(
				"service \"{first}\": network_mode {agreed:?} is incompatible with \
				 x-podman-pod; a pod can only run on pasta or slirp4netns, declared alike \
				 on every service"
			));
		}
		// When every service agrees on pasta/slirp4netns, no service may
		// also declare `networks:`; libpod refuses the combination on a pod.
		for (name, service) in &services {
			if !service.networks.names().is_empty() {
				return Err(format!(
					"service \"{name}\": networks cannot be combined with network_mode \
					 {agreed:?} in x-podman-pod"
				));
			}
		}
	}

	// 2. Divergent networks: the first service that declares any network
	//    defines the canonical set, every other service with a non-empty
	//    `networks:` must equal it. (Skipped when the project agreed on a
	//    pasta/slirp4netns mode, because every service already raised an
	//    error above if any declared a network.)
	let mut canonical: Option<IndexSet<String>> = None;
	for (name, service) in &services {
		let names: IndexSet<String> = service.networks.names().into_iter().collect();
		if names.is_empty() {
			continue;
		}
		match &canonical {
			None => canonical = Some(names),
			Some(c) if c != &names => {
				let c_list: Vec<&str> = c.iter().map(String::as_str).collect();
				let n_list: Vec<&str> = names.iter().map(String::as_str).collect();
				return Err(format!(
					"service \"{name}\": networks: [{n}] does not match the first service's \
					 networks: [{c}]; x-podman-pod requires every service to declare the same \
					 set of networks (or none, for the project default)",
					n = n_list.join(", "),
					c = c_list.join(", "),
				));
			}
			_ => {}
		}
	}

	// 3 & 4. Port collisions: same host_port on more than one service, and
	//    the same host_port bound to two different host IPs. Tracked by
	//    (host_port, protocol) so a TCP and UDP on the same host port do
	//    not falsely collide.
	// A pod has one user namespace and Podman refuses a member with its own,
	// so every service declares the same `userns_mode`, or none.
	let mut userns: Option<(&str, Option<&str>)> = None;
	for (name, service) in &services {
		let mode = service.userns_mode.as_deref();
		match userns {
			None => userns = Some((name, mode)),
			Some((first, first_mode)) if first_mode != mode => {
				return Err(format!(
					"service \"{name}\": userns_mode {} does not match service \"{first}\"'s {}; \
					 x-podman-pod gives the pod one user namespace, so every service declares \
					 the same userns_mode (or none)",
					mode.map_or("(unset)".to_string(), |m| format!("{m:?}")),
					first_mode.map_or("(unset)".to_string(), |m| format!("{m:?}")),
				));
			}
			_ => {}
		}
	}

	let mut host_port_owner: std::collections::HashMap<(u16, String), String> =
		std::collections::HashMap::new();
	for (name, service) in &services {
		let Ok(parsed) = crate::ports::parse_ports(&service.ports) else {
			// An invalid port is already reported by the per-service path; the
			// pod check is not the right place to re-report it.
			continue;
		};
		for p in &parsed {
			let host_port = match p.host_port {
				Some(n) if n != 0 => n,
				_ => continue,
			};
			let key = (host_port, p.protocol.clone());

			// 3. Duplicate host port.
			if let Some(prev) = host_port_owner.get(&key) {
				if prev != name {
					return Err(format!(
						"services \"{prev}\" and \"{name}\" both publish host port \
						 {host_port}/{}; x-podman-pod hands the union of every service's \
						 ports: to the pod, where duplicate host ports collide",
						p.protocol,
					));
				}
				continue;
			}
			host_port_owner.insert(key, name.to_string());
		}
	}

	Ok(())
}
