//! Network creation and per-network options for container specs.

use std::collections::HashMap;

use tracing::info;

use crate::compose::types::{ComposeFile, IpamConfig, Service, ServiceNetworkConfig};
use crate::error::{ComposeError, Result};
use crate::libpod::types::container::{Namespace, PerNetworkOptions};
use crate::libpod::types::network::{NetworkCreateRequest, Subnet};

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

			let subnets = config
				.as_ref()
				.and_then(|c| c.ipam.as_ref())
				.map(build_subnets)
				.unwrap_or_default();

			let request = NetworkCreateRequest {
				name: network_name.clone(),
				driver: Some(driver),
				internal: config.as_ref().and_then(|c| c.internal),
				attachable: config.as_ref().and_then(|c| c.attachable),
				ipv6_enabled: config.as_ref().and_then(|c| c.enable_ipv6),
				dns_enabled: Some(true),
				options: driver_opts,
				labels,
				subnets,
			};

			match self
				.client
				.post_json::<_, serde_json::Value>("/v4.0.0/libpod/networks/create", &request)
				.await
			{
				Ok(_) => info!("created network {network_name}"),
				Err(ref e) if e.is_status(409) => {}
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
		if let Some(prio) = c.priority {
			let mut driver_opts = HashMap::new();
			driver_opts.insert("priority".to_string(), prio.to_string());
			opts.driver_opts = Some(driver_opts);
		}
	} else if let Some(mac) = fallback_mac {
		opts.static_mac = Some(mac.to_string());
	}

	opts
}

pub(super) fn resolve_network_mode(
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
				.map(|s| {
					s.container_name
						.clone()
						.unwrap_or_else(|| format!("{project}-{svc_name}"))
				})
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
				service.networks.config_for(net_name),
				service.mac_address.as_deref(),
			);
			(full, opts)
		})
		.collect();

	(None, networks)
}

/// Resolve the actual network name on the host for a compose network key.
pub(super) fn resolve_network_name(network: &str, file: &ComposeFile, project: &str) -> String {
	file.networks
		.get(network)
		.and_then(|c| c.as_ref())
		.and_then(|c| c.name.as_deref())
		.map(|s| s.to_string())
		.unwrap_or_else(|| format!("{project}_{network}"))
}

fn build_subnets(ipam: &IpamConfig) -> Vec<Subnet> {
	ipam.config
		.iter()
		.map(|pool| Subnet {
			subnet: pool.subnet.clone(),
			gateway: pool.gateway.clone(),
			lease_range: None,
		})
		.collect()
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
	fn resolve_network_mode_explicit_mode() {
		let svc = Service {
			network_mode: Some("host".to_string()),
			..Default::default()
		};
		let file = empty_file();
		let (ns, nets) = resolve_network_mode(&svc, &file, "proj");
		assert!(ns.is_some());
		assert_eq!(ns.unwrap().nsmode, "host");
		assert!(nets.is_empty());
	}

	#[test]
	fn resolve_network_mode_no_networks() {
		let svc = Service::default();
		let file = empty_file();
		let (ns, nets) = resolve_network_mode(&svc, &file, "proj");
		assert!(ns.is_none());
		assert!(nets.is_empty());
	}

	#[test]
	fn build_per_network_options_no_config() {
		let opts = build_per_network_options(None, None);
		assert!(opts.aliases.is_empty());
		assert!(opts.static_ips.is_empty());
	}

	#[test]
	fn build_per_network_options_with_aliases() {
		use crate::compose::types::ServiceNetworkConfig;
		let cfg = ServiceNetworkConfig {
			aliases: Some(vec!["web".to_string(), "api".to_string()]),
			..Default::default()
		};
		let opts = build_per_network_options(Some(&cfg), None);
		assert_eq!(opts.aliases, vec!["web".to_string(), "api".to_string()]);
	}

	#[test]
	fn build_per_network_options_with_ipv4() {
		use crate::compose::types::ServiceNetworkConfig;
		let cfg = ServiceNetworkConfig {
			ipv4_address: Some("10.0.0.5".to_string()),
			..Default::default()
		};
		let opts = build_per_network_options(Some(&cfg), None);
		assert!(opts.static_ips.contains(&"10.0.0.5".to_string()));
	}

	#[test]
	fn fallback_mac_applied_when_no_config() {
		let opts = build_per_network_options(None, Some("02:42:ac:11:00:02"));
		assert_eq!(opts.static_mac.as_deref(), Some("02:42:ac:11:00:02"));
	}

	#[test]
	fn per_network_mac_takes_precedence_over_fallback() {
		use crate::compose::types::ServiceNetworkConfig;
		let cfg = ServiceNetworkConfig {
			mac_address: Some("aa:bb:cc:dd:ee:ff".to_string()),
			..Default::default()
		};
		let opts = build_per_network_options(Some(&cfg), Some("02:42:ac:11:00:03"));
		assert_eq!(opts.static_mac.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
	}
}
