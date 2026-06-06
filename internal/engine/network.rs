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
				.unwrap_or(name);

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
				name: network_name.to_string(),
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
			let full_name = resolve_network_name(network, file);
			let endpoint_config =
				build_endpoint_settings(service.networks.config_for(network), file);
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
// Free helpers (pub(super) so container.rs can call them)
// ---------------------------------------------------------------------------

pub(super) fn build_endpoint_settings(
	cfg: Option<&ServiceNetworkConfig>,
	_file: &ComposeFile,
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
		if c.mac_address.is_some() {
			settings.mac_address = c.mac_address.clone();
		}
		if let Some(prio) = c.priority {
			let mut m = HashMap::new();
			m.insert("priority".to_string(), prio.to_string());
			settings.driver_opts = Some(m);
		}
	}
	settings
}

/// Determine `network_mode` and the first named network for `NetworkingConfig`.
///
/// Returns `(Option<network_mode>, Option<first_network_name>)`.
pub(super) fn resolve_network_mode(
	service: &Service,
	file: &ComposeFile,
) -> (Option<String>, Option<String>) {
	if let Some(mode) = &service.network_mode {
		return (Some(mode.clone()), None);
	}
	let networks = service.networks.names();
	if networks.is_empty() {
		(None, None)
	} else {
		let first = resolve_network_name(&networks[0], file);
		(None, Some(first))
	}
}

pub(super) fn resolve_network_name(network: &str, file: &ComposeFile) -> String {
	file.networks
		.get(network)
		.and_then(|c| c.as_ref())
		.and_then(|c| c.name.as_deref())
		.unwrap_or(network)
		.to_string()
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;
	use crate::compose::types::{ComposeFile, NetworkConfig, Service};
	use indexmap::IndexMap;

	fn empty_file() -> ComposeFile {
		ComposeFile::default()
	}

	fn file_with_named_network(key: &str, name: &str) -> ComposeFile {
		let mut cfg = NetworkConfig::default();
		cfg.name = Some(name.to_string());
		let mut file = empty_file();
		file.networks.insert(key.to_string(), Some(cfg));
		file
	}

	#[test]
	fn resolve_network_name_key_not_found_returns_key() {
		let file = empty_file();
		assert_eq!(resolve_network_name("mynet", &file), "mynet");
	}

	#[test]
	fn resolve_network_name_uses_config_name() {
		let file = file_with_named_network("mynet", "custom-net-name");
		assert_eq!(resolve_network_name("mynet", &file), "custom-net-name");
	}

	#[test]
	fn resolve_network_mode_explicit_mode() {
		let mut svc = Service::default();
		svc.network_mode = Some("host".to_string());
		let file = empty_file();
		let (mode, first) = resolve_network_mode(&svc, &file);
		assert_eq!(mode.as_deref(), Some("host"));
		assert!(first.is_none());
	}

	#[test]
	fn resolve_network_mode_no_networks() {
		let svc = Service::default();
		let file = empty_file();
		let (mode, first) = resolve_network_mode(&svc, &file);
		assert!(mode.is_none());
		assert!(first.is_none());
	}

	#[test]
	fn build_endpoint_settings_no_config() {
		let file = empty_file();
		let settings = build_endpoint_settings(None, &file);
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
		let settings = build_endpoint_settings(Some(&cfg), &file);
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
		let settings = build_endpoint_settings(Some(&cfg), &file);
		let ipam = settings.ipam_config.unwrap();
		assert_eq!(ipam.ipv4_address.as_deref(), Some("10.0.0.5"));
	}
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
