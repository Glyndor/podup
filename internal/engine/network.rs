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
};
use bollard::network::{ConnectNetworkOptions, CreateNetworkOptions};
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
            labels.insert("lynx.compose.project".to_string(), self.project.clone());

            let driver_opts: HashMap<String, String> = config
                .as_ref()
                .map(|c| c.driver_opts.clone())
                .unwrap_or_default();

            let ipam = config
                .as_ref()
                .and_then(|c| c.ipam.as_ref())
                .map(build_ipam);

            let options = CreateNetworkOptions::<String> {
                name: network_name.to_string(),
                driver: driver.clone(),
                internal: config.as_ref().and_then(|c| c.internal).unwrap_or(false),
                attachable: config.as_ref().and_then(|c| c.attachable).unwrap_or(false),
                enable_ipv6: config.as_ref().and_then(|c| c.enable_ipv6).unwrap_or(false),
                options: driver_opts,
                labels,
                ipam: ipam.unwrap_or_default(),
                ..Default::default()
            };

            match self.docker.create_network(options).await {
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
                    ConnectNetworkOptions {
                        container: container_name,
                        endpoint_config,
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
