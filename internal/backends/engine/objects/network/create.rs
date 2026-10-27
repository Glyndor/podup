//! Network creation on the host, and the label that records who owns it.

use std::collections::HashMap;

use crate::compose::types::ComposeFile;
use crate::engine::Engine;
use crate::error::{ComposeError, Result};
use crate::libpod::types::network::NetworkCreateRequest;
use crate::libpod::API_PREFIX;

use super::super::ownership::Owner;
use super::ipam::{build_ipam_options, build_subnets};
use super::isolation::apply_default_isolation;

impl Engine {
	/// Pre-create every declared (non-external) network before containers start,
	/// stamping each with the `podup.project` label and applying driver/IPAM/label
	/// config. External networks are verified to already exist instead.
	///
	/// An already-exists conflict (libpod returns 409) is treated as success on
	/// re-`up` *only* when the existing network is labelled for this project.
	/// An existing network labelled for a different project, or carrying no
	/// `podup.project` label at all, is refused: a project that wants to share
	/// a network says so by declaring `external: true`. The project name is the
	/// isolation boundary; silently joining another project's bridge would let
	/// this project's containers reach services on the other project through
	/// DNS, not just by IP.
	pub(in crate::backends::engine) async fn create_networks(
		&self,
		file: &ComposeFile,
	) -> Result<()> {
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

			let mut driver_opts: HashMap<String, String> = config
				.as_ref()
				.map(|c| c.driver_opts.clone())
				.unwrap_or_default();
			apply_default_isolation(&driver, &mut driver_opts);

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

			crate::ui::progress::start("Network", &network_name, "Creating");
			match self
				.client
				.post_json::<_, serde_json::Value>(
					&format!("{API_PREFIX}/networks/create"),
					&request,
				)
				.await
			{
				Ok(_) => crate::ui::progress_line("Network", &network_name, "Created"),
				// An already-exists conflict on re-`up` is success only when
				// the existing network is ours. Anything else is a project-
				// boundary violation: refuse with the same error shape
				// regardless of whether the existing network carries a
				// foreign `podup.project` label or no label at all (the
				// label is the only ownership evidence; "no one owns it"
				// and "another stack already claimed it" are
				// indistinguishable). The row still needs to close on the
				// accepted path: without an explicit closing verb the live
				// board leaves it spinning on `Creating` (#1347).
				Err(ref e) if e.is_already_exists() => {
					match self.inspect_network_owner(&network_name).await? {
						Owner::Project(owner) if owner == self.project => {
							crate::ui::progress_line("Network", &network_name, "Exists");
						}
						Owner::Project(owner) => {
							crate::ui::progress_line("Network", &network_name, "Failed");
							return Err(ComposeError::Unsupported(format!(
								"network '{network_name}' already exists and is labelled \
								 podup.project={owner}; refusing to attach this project \
								 ('{}') to it. The compose file must declare \
								 'networks.{name}.external: true' to share the network.",
								self.project
							)));
						}
						Owner::Unlabelled | Owner::Absent => {
							crate::ui::progress_line("Network", &network_name, "Failed");
							return Err(ComposeError::Unsupported(format!(
								"network '{network_name}' already exists and carries no \
								 podup.project label; refusing to attach this project \
								 ('{}') to it without ownership evidence. The compose \
								 file must declare 'networks.{name}.external: true' to \
								 share an existing unlabelled network.",
								self.project
							)));
						}
					}
				}
				Err(e) => return Err(ComposeError::Podman(e)),
			}
		}
		Ok(())
	}

	/// Who owns the network named `name` on the host: absent, present without
	/// a `podup.project` label, or labelled for a project. Transport failures
	/// propagate as `ComposeError::Podman`.
	///
	/// The label is the only ownership evidence on a podup-created network,
	/// so this is what both `create_networks` (on a 409) and the file-driven
	/// `down` path consult before reusing or removing an existing network.
	pub(in crate::backends::engine) async fn inspect_network_owner(
		&self,
		name: &str,
	) -> Result<Owner> {
		self.object_owner("networks", name, &["/labels", "/Labels"])
			.await
	}
}

#[cfg(test)]
#[path = "create_tests.rs"]
mod tests;
