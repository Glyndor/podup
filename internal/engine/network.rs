//! Network creation and per-network options for container specs.

use std::collections::HashMap;

use tracing::info;

use crate::compose::types::{ComposeFile, IpamConfig, Service, ServiceNetworkConfig};
use crate::error::{ComposeError, Result};
use crate::libpod::types::container::{Namespace, PerNetworkOptions};
use crate::libpod::types::network::{LeaseRange, NetworkCreateRequest, Subnet};
use crate::libpod::API_PREFIX;

use super::Engine;

impl Engine {
	pub(super) async fn create_networks(&self, file: &ComposeFile) -> Result<()> {
		for (name, config) in &file.networks {
			let network_name = config
				.as_ref()
				.and_then(|c| c.name.as_deref())
				.map(|s| s.to_string())
				.unwrap_or_else(|| format!("{}_{}", self.project, name));

			let external = config.as_ref().and_then(|c| c.external).unwrap_or(false);
			if external {
				let external_name = config
					.as_ref()
					.and_then(|c| c.name.as_deref())
					.unwrap_or(name);
				self.ensure_external_exists("network", "networks", external_name)
					.await?;
				continue;
			}

			let driver = config
				.as_ref()
				.and_then(|c| c.driver.clone())
				.unwrap_or_else(|| "bridge".into());

			let mut labels: HashMap<String, String> = config
				.as_ref()
				.map(|c| c.labels.to_map())
				.unwrap_or_default();
			labels.insert("podup.project".to_string(), self.project.clone());

			let driver_opts: HashMap<String, String> = config
				.as_ref()
				.map(|c| c.driver_opts.clone())
				.unwrap_or_default();

			let ipam = config.as_ref().and_then(|c| c.ipam.as_ref());
			let subnets = ipam.map(build_subnets).unwrap_or_default();
			let ipam_options = ipam.map(build_ipam_options).unwrap_or_default();

			let request = NetworkCreateRequest {
				name: network_name.clone(),
				driver: Some(driver),
				internal: config.as_ref().and_then(|c| c.internal),
				attachable: config.as_ref().and_then(|c| c.attachable),
				ipv6_enabled: config.as_ref().and_then(|c| c.enable_ipv6),
				dns_enabled: Some(true),
				options: driver_opts,
				ipam_options,
				labels,
				subnets,
			};

			match self
				.client
				.post_json::<_, serde_json::Value>(
					&format!("{API_PREFIX}/networks/create"),
					&request,
				)
				.await
			{
				Ok(_) => info!("created network {network_name}"),
				// An existing network is not an error on re-`up`; accept any
				// already-exists conflict (network-create returns 409, but share
				// the same predicate as volume-create for consistency).
				Err(ref e) if e.is_already_exists() => {}
				Err(e) => return Err(ComposeError::Podman(e)),
			}
		}
		Ok(())
	}
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

pub(super) fn build_per_network_options(
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
		if c.gw_priority.is_some() {
			tracing::debug!("network gw_priority is not supported by Podman and is ignored");
		}
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

pub(super) fn resolve_network_mode(
	service_name: &str,
	service: &Service,
	file: &ComposeFile,
	project: &str,
) -> (Option<Namespace>, HashMap<String, PerNetworkOptions>) {
	if let Some(mode) = &service.network_mode {
		let ns = if let Some(id) = mode.strip_prefix("container:") {
			Namespace::container(id)
		} else if let Some(svc_name) = mode.strip_prefix("service:") {
			let cname = file
				.services
				.get(svc_name)
				.map(|s| resolve_target_container_name(svc_name, s, project))
				.unwrap_or_else(|| svc_name.to_string());
			Namespace::container(cname)
		} else {
			Namespace::new(mode)
		};
		return (Some(ns), HashMap::new());
	}

	let network_names = service.networks.names();
	let networks: HashMap<String, PerNetworkOptions> = network_names
		.iter()
		.map(|net_name| {
			let full = resolve_network_name(net_name, file, project);
			let opts = build_per_network_options(
				service_name,
				service.networks.config_for(net_name),
				service.mac_address.as_deref(),
			);
			(full, opts)
		})
		.collect();

	// Podman libpod requires netns=bridge when explicit networks are used.
	let netns = (!networks.is_empty()).then(|| Namespace::new("bridge"));

	(netns, networks)
}

/// Resolve the container name that a `network_mode: service:<name>` reference
/// should attach to.
///
/// An explicit `container_name` is honoured verbatim. Otherwise the name is
/// derived from the project + service. A scaled service (replicas > 1 via
/// `deploy.replicas` or `scale:`) has no base-named container — its replicas are
/// `{project}-{svc}-1..N` — so we point at replica `-1`, matching docker-compose,
/// which attaches `network_mode: service:` references to the first replica.
fn resolve_target_container_name(svc_name: &str, service: &Service, project: &str) -> String {
	if let Some(name) = &service.container_name {
		return name.clone();
	}
	let base = format!("{project}-{svc_name}");
	if service_replicas(service) > 1 {
		format!("{base}-1")
	} else {
		base
	}
}

/// Number of replicas a service declares in the compose file, via `scale:` or
/// `deploy.replicas`. Defaults to 1. Does not consult CLI `--scale` overrides.
fn service_replicas(service: &Service) -> u32 {
	service
		.scale
		.or(service.deploy.as_ref().and_then(|d| d.replicas))
		.unwrap_or(1)
}

/// Resolve the actual network name on the host for a compose network key.
pub(super) fn resolve_network_name(network: &str, file: &ComposeFile, project: &str) -> String {
	match file.networks.get(network).and_then(|c| c.as_ref()) {
		Some(cfg) => {
			if let Some(name) = cfg.name.as_deref() {
				name.to_string()
			} else if cfg.external.unwrap_or(false) {
				network.to_string()
			} else {
				format!("{project}_{network}")
			}
		}
		None => format!("{project}_{network}"),
	}
}

fn build_subnets(ipam: &IpamConfig) -> Vec<Subnet> {
	ipam.config
		.iter()
		.map(|pool| {
			if !pool.aux_addresses.is_empty() {
				tracing::warn!(
					"ipam aux_addresses are not supported by Podman and will be ignored"
				);
			}
			Subnet {
				subnet: pool.subnet.clone(),
				gateway: pool.gateway.clone(),
				lease_range: pool.ip_range.as_deref().and_then(lease_range_from_cidr),
			}
		})
		.collect()
}

/// Translate `ipam.driver` and `ipam.options` into Podman's `ipam_options` map.
fn build_ipam_options(ipam: &IpamConfig) -> HashMap<String, String> {
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

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;
	use crate::compose::types::{ComposeFile, NetworkConfig, Service};

	fn empty_file() -> ComposeFile {
		ComposeFile::default()
	}

	fn file_with_named_network(key: &str, name: &str) -> ComposeFile {
		let cfg = NetworkConfig {
			name: Some(name.to_string()),
			..Default::default()
		};
		let mut file = empty_file();
		file.networks.insert(key.to_string(), Some(cfg));
		file
	}

	#[test]
	fn resolve_network_name_key_not_found_prefixes_project() {
		let file = empty_file();
		assert_eq!(resolve_network_name("mynet", &file, "proj"), "proj_mynet");
	}

	#[test]
	fn resolve_network_name_uses_config_name_over_prefix() {
		let file = file_with_named_network("mynet", "custom-net-name");
		assert_eq!(
			resolve_network_name("mynet", &file, "proj"),
			"custom-net-name"
		);
	}

	#[test]
	fn resolve_network_name_external_uses_key_not_prefix() {
		let cfg = NetworkConfig {
			external: Some(true),
			..Default::default()
		};
		let mut file = empty_file();
		file.networks.insert("shared".to_string(), Some(cfg));
		assert_eq!(resolve_network_name("shared", &file, "proj"), "shared");
	}

	#[test]
	fn resolve_network_mode_explicit_mode() {
		let svc = Service {
			network_mode: Some("host".to_string()),
			..Default::default()
		};
		let file = empty_file();
		let (ns, nets) = resolve_network_mode("web", &svc, &file, "proj");
		assert!(ns.is_some());
		assert_eq!(ns.unwrap().nsmode, "host");
		assert!(nets.is_empty());
	}

	fn file_with_service(svc_name: &str, svc: Service) -> ComposeFile {
		let mut file = empty_file();
		file.services.insert(svc_name.to_string(), svc);
		file
	}

	#[test]
	fn network_mode_service_single_replica_uses_base_name() {
		let target = Service::default();
		let file = file_with_service("db", target);
		let svc = Service {
			network_mode: Some("service:db".to_string()),
			..Default::default()
		};
		let (ns, _) = resolve_network_mode("web", &svc, &file, "proj");
		let ns = ns.unwrap();
		assert_eq!(ns.nsmode, "container");
		assert_eq!(ns.value.as_deref(), Some("proj-db"));
	}

	#[test]
	fn network_mode_service_scaled_replicas_resolves_replica_one() {
		// `scale:`/`deploy.replicas` > 1 means the base name does not exist —
		// docker-compose attaches to replica `-1`.
		let target = Service {
			scale: Some(3),
			..Default::default()
		};
		let file = file_with_service("db", target);
		let svc = Service {
			network_mode: Some("service:db".to_string()),
			..Default::default()
		};
		let (ns, _) = resolve_network_mode("web", &svc, &file, "proj");
		assert_eq!(ns.unwrap().value.as_deref(), Some("proj-db-1"));
	}

	#[test]
	fn network_mode_service_deploy_replicas_resolves_replica_one() {
		use crate::compose::types::DeployConfig;
		let target = Service {
			deploy: Some(DeployConfig {
				replicas: Some(2),
				..Default::default()
			}),
			..Default::default()
		};
		let file = file_with_service("db", target);
		let svc = Service {
			network_mode: Some("service:db".to_string()),
			..Default::default()
		};
		let (ns, _) = resolve_network_mode("web", &svc, &file, "proj");
		assert_eq!(ns.unwrap().value.as_deref(), Some("proj-db-1"));
	}

	#[test]
	fn network_mode_service_container_name_wins_over_replica() {
		// An explicit container_name is honoured verbatim even when scaled.
		let target = Service {
			scale: Some(4),
			container_name: Some("custom-db".to_string()),
			..Default::default()
		};
		let file = file_with_service("db", target);
		let svc = Service {
			network_mode: Some("service:db".to_string()),
			..Default::default()
		};
		let (ns, _) = resolve_network_mode("web", &svc, &file, "proj");
		assert_eq!(ns.unwrap().value.as_deref(), Some("custom-db"));
	}

	#[test]
	fn network_mode_service_unknown_target_uses_raw_name() {
		let file = empty_file();
		let svc = Service {
			network_mode: Some("service:missing".to_string()),
			..Default::default()
		};
		let (ns, _) = resolve_network_mode("web", &svc, &file, "proj");
		assert_eq!(ns.unwrap().value.as_deref(), Some("missing"));
	}

	#[test]
	fn resolve_network_mode_no_networks() {
		let svc = Service::default();
		let file = empty_file();
		let (ns, nets) = resolve_network_mode("web", &svc, &file, "proj");
		assert!(ns.is_none());
		assert!(nets.is_empty());
	}

	#[test]
	fn build_per_network_options_seeds_service_name_alias() {
		// With no explicit config, the service name is still registered as an
		// alias so siblings can reach the service by name.
		let opts = build_per_network_options("web", None, None);
		assert_eq!(opts.aliases, vec!["web".to_string()]);
		assert!(opts.static_ips.is_empty());
	}

	#[test]
	fn build_per_network_options_empty_service_name_adds_no_alias() {
		let opts = build_per_network_options("", None, None);
		assert!(opts.aliases.is_empty());
	}

	#[test]
	fn build_per_network_options_with_aliases() {
		use crate::compose::types::ServiceNetworkConfig;
		let cfg = ServiceNetworkConfig {
			aliases: Some(vec!["api".to_string()]),
			..Default::default()
		};
		// The service name is prepended ahead of any explicit aliases.
		let opts = build_per_network_options("web", Some(&cfg), None);
		assert_eq!(opts.aliases, vec!["web".to_string(), "api".to_string()]);
	}

	#[test]
	fn build_per_network_options_does_not_duplicate_service_name() {
		use crate::compose::types::ServiceNetworkConfig;
		let cfg = ServiceNetworkConfig {
			aliases: Some(vec!["web".to_string(), "api".to_string()]),
			..Default::default()
		};
		// An explicit alias equal to the service name is not duplicated.
		let opts = build_per_network_options("web", Some(&cfg), None);
		assert_eq!(opts.aliases, vec!["web".to_string(), "api".to_string()]);
	}

	#[test]
	fn build_per_network_options_with_ipv4() {
		use crate::compose::types::ServiceNetworkConfig;
		let cfg = ServiceNetworkConfig {
			ipv4_address: Some("10.0.0.5".to_string()),
			..Default::default()
		};
		let opts = build_per_network_options("web", Some(&cfg), None);
		assert!(opts.static_ips.contains(&"10.0.0.5".to_string()));
	}

	#[test]
	fn fallback_mac_applied_when_no_config() {
		let opts = build_per_network_options("web", None, Some("02:42:ac:11:00:02"));
		assert_eq!(opts.static_mac.as_deref(), Some("02:42:ac:11:00:02"));
	}

	#[test]
	fn lease_range_ipv4_reserves_network_and_broadcast() {
		let lr = lease_range_from_cidr("172.28.5.0/24").unwrap();
		assert_eq!(lr.start_ip.as_deref(), Some("172.28.5.1"));
		assert_eq!(lr.end_ip.as_deref(), Some("172.28.5.254"));
	}

	#[test]
	fn lease_range_ipv4_slash31_uses_both_addresses() {
		let lr = lease_range_from_cidr("10.0.0.0/31").unwrap();
		assert_eq!(lr.start_ip.as_deref(), Some("10.0.0.0"));
		assert_eq!(lr.end_ip.as_deref(), Some("10.0.0.1"));
	}

	#[test]
	fn lease_range_ipv6_full_span() {
		let lr = lease_range_from_cidr("2001:db8::/120").unwrap();
		assert_eq!(lr.start_ip.as_deref(), Some("2001:db8::"));
		assert_eq!(lr.end_ip.as_deref(), Some("2001:db8::ff"));
	}

	#[test]
	fn lease_range_invalid_cidr_is_none() {
		assert!(lease_range_from_cidr("not-a-cidr").is_none());
		assert!(lease_range_from_cidr("10.0.0.0/40").is_none());
	}

	#[test]
	fn ipam_options_include_driver_and_options() {
		use crate::compose::types::IpamConfig;
		let ipam = IpamConfig {
			driver: Some("host-local".into()),
			options: [("foo".to_string(), "bar".to_string())].into(),
			..Default::default()
		};
		let opts = build_ipam_options(&ipam);
		assert_eq!(opts.get("driver").map(String::as_str), Some("host-local"));
		assert_eq!(opts.get("foo").map(String::as_str), Some("bar"));
	}

	#[test]
	fn per_network_interface_name_forwarded() {
		use crate::compose::types::ServiceNetworkConfig;
		let cfg = ServiceNetworkConfig {
			interface_name: Some("eth1".into()),
			..Default::default()
		};
		let opts = build_per_network_options("web", Some(&cfg), None);
		assert_eq!(opts.interface_name.as_deref(), Some("eth1"));
	}

	#[test]
	fn per_network_mac_takes_precedence_over_fallback() {
		use crate::compose::types::ServiceNetworkConfig;
		let cfg = ServiceNetworkConfig {
			mac_address: Some("aa:bb:cc:dd:ee:ff".to_string()),
			..Default::default()
		};
		let opts = build_per_network_options("web", Some(&cfg), Some("02:42:ac:11:00:03"));
		assert_eq!(opts.static_mac.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
	}
}
