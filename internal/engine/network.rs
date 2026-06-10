//! Network creation and service attachment.
//!
//! [`Engine::create_networks`] creates all non-external networks declared in
//! the compose file before any containers start. [`Engine::connect_extra_networks`]
//! attaches a running container to any additional networks beyond its primary
//! one (Docker API creates containers connected to only one network; extras need
//! a separate `ConnectNetwork` call).

use std::collections::HashMap;

use bollard::models::{
	EndpointIpamConfig, EndpointSettings, Ipam, IpamConfig as BollardIpamConfig,
	NetworkConnectRequest, NetworkCreateRequest,
};
use tracing::{debug, info};

use crate::compose::types::{ComposeFile, IpamConfig, Service, ServiceNetworkConfig};
use crate::error::{ComposeError, Result};

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

			let ipam = config
				.as_ref()
				.and_then(|c| c.ipam.as_ref())
				.map(build_ipam);

			let request = NetworkCreateRequest {
				name: network_name.clone(),
				driver: Some(driver.clone()),
				internal: config.as_ref().and_then(|c| c.internal),
				attachable: config.as_ref().and_then(|c| c.attachable),
				enable_ipv6: config.as_ref().and_then(|c| c.enable_ipv6),
				options: if driver_opts.is_empty() {
					None
				} else {
					Some(driver_opts)
				},
				labels: if labels.is_empty() {
					None
				} else {
					Some(labels)
				},
				ipam,
				..Default::default()
			};

			match self.docker.create_network(request).await {
				Ok(_) => info!("created network {network_name}"),
				Err(bollard::errors::Error::DockerResponseServerError {
					status_code: 409, ..
				}) => {}
				Err(e) => return Err(ComposeError::Podman(e)),
			}
		}
		Ok(())
	}

	pub(super) async fn connect_extra_networks(
		&self,
		container_name: &str,
		service: &Service,
		file: &ComposeFile,
	) -> Result<()> {
		if service.network_mode.is_some() {
			return Ok(());
		}

		let network_names = service.networks.names();
		for network in network_names.iter().skip(1) {
			let full_name = resolve_network_name(network, file, &self.project);
			let endpoint_config =
				build_endpoint_settings(service.networks.config_for(network), file, None);
			self.docker
				.connect_network(
					&full_name,
					NetworkConnectRequest {
						container: container_name.to_string(),
						endpoint_config: Some(endpoint_config),
					},
				)
				.await?;
			debug!("connected {container_name} to network {full_name}");
		}

		Ok(())
	}
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

pub(super) fn build_endpoint_settings(
	cfg: Option<&ServiceNetworkConfig>,
	_file: &ComposeFile,
	fallback_mac: Option<&str>,
) -> EndpointSettings {
	let mut settings = EndpointSettings::default();
	if let Some(c) = cfg {
		if let Some(aliases) = &c.aliases {
			settings.aliases = Some(aliases.clone());
		}
		if c.ipv4_address.is_some() || c.ipv6_address.is_some() || !c.link_local_ips.is_empty() {
			settings.ipam_config = Some(EndpointIpamConfig {
				ipv4_address: c.ipv4_address.clone(),
				ipv6_address: c.ipv6_address.clone(),
				link_local_ips: if c.link_local_ips.is_empty() {
					None
				} else {
					Some(c.link_local_ips.clone())
				},
			});
		}
		let mac = c.mac_address.as_deref().or(fallback_mac);
		if mac.is_some() {
			settings.mac_address = mac.map(|s| s.to_string());
		}
		if let Some(prio) = c.priority {
			let mut m = HashMap::new();
			m.insert("priority".to_string(), prio.to_string());
			settings.driver_opts = Some(m);
		}
	} else if fallback_mac.is_some() {
		settings.mac_address = fallback_mac.map(|s| s.to_string());
	}
	settings
}

pub(super) fn resolve_network_mode(
	service: &Service,
	file: &ComposeFile,
	project: &str,
) -> (Option<String>, Option<String>) {
	if let Some(mode) = &service.network_mode {
		return (Some(mode.clone()), None);
	}
	let networks = service.networks.names();
	if networks.is_empty() {
		(None, None)
	} else {
		let first = resolve_network_name(&networks[0], file, project);
		(None, Some(first))
	}
}

/// Resolve the actual network name on the host for a compose network key.
///
/// If the network config has an explicit `name:`, that takes precedence.
/// Otherwise the name is prefixed with `{project}_` to avoid collisions
/// between projects that use the same compose key (e.g. "default").
pub(super) fn resolve_network_name(network: &str, file: &ComposeFile, project: &str) -> String {
	file.networks
		.get(network)
		.and_then(|c| c.as_ref())
		.and_then(|c| c.name.as_deref())
		.map(|s| s.to_string())
		.unwrap_or_else(|| format!("{project}_{network}"))
}

fn build_ipam(ipam: &IpamConfig) -> Ipam {
	let config = if ipam.config.is_empty() {
		None
	} else {
		Some(
			ipam.config
				.iter()
				.map(|pool| BollardIpamConfig {
					subnet: pool.subnet.clone(),
					gateway: pool.gateway.clone(),
					ip_range: pool.ip_range.clone(),
					auxiliary_addresses: if pool.aux_addresses.is_empty() {
						None
					} else {
						Some(pool.aux_addresses.clone())
					},
				})
				.collect(),
		)
	};

	Ipam {
		driver: ipam.driver.clone(),
		config,
		options: if ipam.options.is_empty() {
			None
		} else {
			Some(ipam.options.clone())
		},
	}
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
		let (mode, first) = resolve_network_mode(&svc, &file, "proj");
		assert_eq!(mode.as_deref(), Some("host"));
		assert!(first.is_none());
	}

	#[test]
	fn resolve_network_mode_no_networks() {
		let svc = Service::default();
		let file = empty_file();
		let (mode, first) = resolve_network_mode(&svc, &file, "proj");
		assert!(mode.is_none());
		assert!(first.is_none());
	}

	#[test]
	fn build_endpoint_settings_no_config() {
		let file = empty_file();
		let settings = build_endpoint_settings(None, &file, None);
		assert!(settings.aliases.is_none());
		assert!(settings.ipam_config.is_none());
	}

	#[test]
	fn build_endpoint_settings_with_aliases() {
		use crate::compose::types::ServiceNetworkConfig;
		let cfg = ServiceNetworkConfig {
			aliases: Some(vec!["web".to_string(), "api".to_string()]),
			..Default::default()
		};
		let file = empty_file();
		let settings = build_endpoint_settings(Some(&cfg), &file, None);
		assert_eq!(
			settings.aliases.as_ref().unwrap(),
			&vec!["web".to_string(), "api".to_string()]
		);
	}

	#[test]
	fn build_endpoint_settings_with_ipv4() {
		use crate::compose::types::ServiceNetworkConfig;
		let cfg = ServiceNetworkConfig {
			ipv4_address: Some("10.0.0.5".to_string()),
			..Default::default()
		};
		let file = empty_file();
		let settings = build_endpoint_settings(Some(&cfg), &file, None);
		let ipam = settings.ipam_config.unwrap();
		assert_eq!(ipam.ipv4_address.as_deref(), Some("10.0.0.5"));
	}

	// --- build_ipam ---

	#[test]
	fn build_ipam_defaults_empty() {
		use crate::compose::types::IpamConfig;
		let result = build_ipam(&IpamConfig::default());
		assert!(result.config.is_none());
		assert!(result.options.is_none());
		assert!(result.driver.is_none());
	}

	#[test]
	fn build_ipam_with_driver_and_options() {
		use crate::compose::types::IpamConfig;
		let mut ipam = IpamConfig {
			driver: Some("default".into()),
			..Default::default()
		};
		ipam.options.insert("route_metric".into(), "100".into());
		let result = build_ipam(&ipam);
		assert_eq!(result.driver.as_deref(), Some("default"));
		assert!(result.options.is_some());
	}

	#[test]
	fn build_ipam_with_subnet_pool() {
		use crate::compose::types::{IpamConfig, IpamPool};
		let pool = IpamPool {
			subnet: Some("192.168.0.0/24".into()),
			gateway: Some("192.168.0.1".into()),
			ip_range: Some("192.168.0.128/25".into()),
			aux_addresses: Default::default(),
		};
		let ipam = IpamConfig {
			config: vec![pool],
			..Default::default()
		};
		let result = build_ipam(&ipam);
		let cfg = result.config.unwrap();
		assert_eq!(cfg.len(), 1);
		assert_eq!(cfg[0].subnet.as_deref(), Some("192.168.0.0/24"));
		assert_eq!(cfg[0].gateway.as_deref(), Some("192.168.0.1"));
		assert_eq!(cfg[0].ip_range.as_deref(), Some("192.168.0.128/25"));
		assert!(cfg[0].auxiliary_addresses.is_none());
	}

	#[test]
	fn build_ipam_with_aux_addresses() {
		use crate::compose::types::{IpamConfig, IpamPool};
		let mut pool = IpamPool::default();
		pool.aux_addresses
			.insert("router".into(), "192.168.0.254".into());
		let ipam = IpamConfig {
			config: vec![pool],
			..Default::default()
		};
		let result = build_ipam(&ipam);
		let cfg = result.config.unwrap();
		let aux = cfg[0].auxiliary_addresses.as_ref().unwrap();
		assert_eq!(aux.get("router").map(|s| s.as_str()), Some("192.168.0.254"));
	}

	// --- build_endpoint_settings: fallback_mac ---

	#[test]
	fn fallback_mac_applied_when_no_config() {
		let file = empty_file();
		let settings = build_endpoint_settings(None, &file, Some("02:42:ac:11:00:02"));
		assert_eq!(settings.mac_address.as_deref(), Some("02:42:ac:11:00:02"));
	}

	#[test]
	fn fallback_mac_applied_when_config_has_no_mac() {
		use crate::compose::types::ServiceNetworkConfig;
		let cfg = ServiceNetworkConfig::default();
		let file = empty_file();
		let settings = build_endpoint_settings(Some(&cfg), &file, Some("02:42:ac:11:00:03"));
		assert_eq!(settings.mac_address.as_deref(), Some("02:42:ac:11:00:03"));
	}

	#[test]
	fn per_network_mac_takes_precedence_over_fallback() {
		use crate::compose::types::ServiceNetworkConfig;
		let cfg = ServiceNetworkConfig {
			mac_address: Some("aa:bb:cc:dd:ee:ff".to_string()),
			..Default::default()
		};
		let file = empty_file();
		let settings = build_endpoint_settings(Some(&cfg), &file, Some("02:42:ac:11:00:03"));
		assert_eq!(settings.mac_address.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
	}
}
