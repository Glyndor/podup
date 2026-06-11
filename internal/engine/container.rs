//! Container creation and start: assembles a libpod `SpecGenerator` from a
//! [`Service`] and starts the container.

use std::collections::HashMap;
use std::path::Path;

use crate::compose::types::{ComposeFile, Service};
use crate::error::{ComposeError, Result};
use crate::libpod::types::container::{LinuxResources, Namespace, SpecGenerator};
use crate::libpod::urlencoded;
use crate::{env_file, ports, size};

use super::container_config::{
	build_healthcheck, build_log_config, build_resource_limits, build_restart_policy, build_ulimits,
};
use super::container_misc::{
	build_blkio_config, build_label_file_labels, parse_device, warn_swarm_only_deploy,
};
use super::network::resolve_network_mode;
use super::volume_mounts::build_mounts_all;
use super::Engine;

impl Engine {
	pub(super) async fn create_and_start(
		&self,
		container_name: &str,
		service_name: &str,
		service: &Service,
		file: &ComposeFile,
	) -> Result<()> {
		let derived_image;
		let image: &str = if let Some(img) = service.image.as_deref() {
			img
		} else if service.build.is_some() {
			derived_image = format!("{}:latest", service_name);
			&derived_image
		} else {
			return Err(ComposeError::NoImageOrBuild(service_name.into()));
		};

		warn_swarm_only_deploy(service_name, service);

		if service.gpus.is_some() {
			tracing::warn!(
				"service \"{service_name}\": top-level gpus: is not yet supported \
				— use deploy.resources.reservations.devices for GPU access"
			);
		}

		// --- Environment ---
		let env: HashMap<String, String> = build_env(service, &self.base_dir)?
			.into_iter()
			.filter_map(|s| {
				let idx = s.find('=')?;
				Some((s[..idx].to_string(), s[idx + 1..].to_string()))
			})
			.collect();

		// --- Secrets and configs become bind mounts ---
		let secret_binds = self.build_secret_binds(service, file)?;
		let config_binds = self.build_config_binds(service, file)?;
		let (mounts, named_volumes) =
			build_mounts_all(service, &self.base_dir, &secret_binds, &config_binds);

		// --- Port mappings ---
		let parsed_ports = ports::parse_ports(&service.ports)?;
		let portmappings = ports::to_libpod(&parsed_ports);

		// expose map: port_num → protocol
		let mut expose: HashMap<u16, String> = parsed_ports
			.iter()
			.map(|p| (p.container_port, p.protocol.clone()))
			.collect();
		for raw in &service.expose {
			let (port_str, proto) = if let Some(idx) = raw.rfind('/') {
				(&raw[..idx], raw[idx + 1..].to_string())
			} else {
				(raw.as_str(), "tcp".to_string())
			};
			if let Ok(p) = port_str.parse::<u16>() {
				expose.entry(p).or_insert(proto);
			}
		}

		// --- Restart policy ---
		let (restart_policy, restart_tries) = build_restart_policy(service);

		// --- Logging ---
		let log_configuration = build_log_config(service.logging.as_ref());

		// --- Networks ---
		let (netns, networks) = resolve_network_mode(service, file, &self.project);

		// --- Labels ---
		let label_file_labels = build_label_file_labels(service, &self.base_dir);
		let mut labels = service.labels.to_map();
		for (k, v) in label_file_labels {
			labels.entry(k).or_insert(v);
		}
		if let Some(deploy) = &service.deploy {
			for (k, v) in deploy.labels.to_map() {
				labels.entry(k).or_insert(v);
			}
		}
		labels.insert("podup.project".to_string(), self.project.clone());
		labels.insert("podup.service".to_string(), service_name.to_string());

		// annotations
		let annotations: HashMap<String, String> = service.annotations.to_map();

		// --- Sysctls ---
		let sysctl: HashMap<String, String> = service.sysctls.to_map();

		// --- Resource limits ---
		let mut resource_limits = build_resource_limits(service);
		if let Some(blkio) = build_blkio_config(service) {
			resource_limits
				.get_or_insert_with(LinuxResources::default)
				.block_io = Some(blkio);
		}

		// --- Ulimits ---
		let ulimits = build_ulimits(service);

		// --- Devices ---
		let devices: Vec<_> = service.devices.iter().map(|s| parse_device(s)).collect();

		// --- Namespace modes ---
		let pidns = service.pid.as_deref().map(Namespace::parse);
		let ipcns = service.ipc.as_deref().map(Namespace::parse);
		let utsns = service.uts.as_deref().map(Namespace::parse);
		let cgroupns = service.cgroup.as_deref().map(Namespace::parse);
		let userns = service.userns_mode.as_deref().map(Namespace::parse);

		// --- Platform → os / arch ---
		let (image_os, image_arch) = service
			.platform
			.as_deref()
			.and_then(|p| p.split_once('/'))
			.map(|(os, arch)| (Some(os.to_string()), Some(arch.to_string())))
			.unwrap_or((None, None));

		// --- Links ---
		let mut links: Vec<String> = service.links.clone();
		links.extend_from_slice(&service.external_links);

		// --- SHM size ---
		let shm_size = service.shm_size.as_deref().and_then(size::parse_memory);

		// --- Stop timeout ---
		let stop_timeout = service
			.stop_grace_period
			.as_deref()
			.and_then(size::parse_duration_secs);

		if service.mac_address.is_some() {
			tracing::warn!(
				"service \"{service_name}\": top-level mac_address is deprecated; \
				move it to networks.<network>.mac_address"
			);
		}

		let spec = SpecGenerator {
			name: container_name.to_string(),
			image: image.to_string(),
			command: service.command.as_ref().map(|c| c.to_exec()),
			entrypoint: service.entrypoint.as_ref().map(|c| c.to_exec()),
			env,
			terminal: service.tty,
			stdin: service.stdin_open,
			user: service.user.clone(),
			work_dir: service.working_dir.clone(),
			stop_signal: service.stop_signal.clone(),
			stop_timeout,
			hostname: service.hostname.clone(),
			domainname: service.domainname.clone(),
			labels,
			annotations,
			cap_add: service.cap_add.clone(),
			cap_drop: service.cap_drop.clone(),
			privileged: service.privileged,
			read_only_filesystem: service.read_only,
			security_opt: service.security_opt.clone(),
			sysctl,
			expose,
			portmappings,
			networks,
			netns,
			extra_hosts: service.extra_hosts.clone(),
			dns_server: service.dns.to_list(),
			dns_search: service.dns_search.to_list(),
			dns_option: service.dns_opt.to_list(),
			mounts,
			volumes: named_volumes,
			volumes_from: service.volumes_from.clone(),
			userns,
			pidns,
			ipcns,
			utsns,
			cgroupns,
			cgroup_parent: service.cgroup_parent.clone(),
			resource_limits,
			ulimits,
			shm_size,
			healthconfig: service.healthcheck.as_ref().map(build_healthcheck),
			log_configuration,
			init: service.init,
			restart_policy,
			restart_tries,
			devices,
			device_cgroup_rule: service.device_cgroup_rules.clone(),
			groups: service.group_add.clone(),
			oom_score_adj: service.oom_score_adj,
			runtime: service.runtime.clone(),
			links,
			image_os,
			image_arch,
			storage_opts: service.storage_opt.clone(),
			..Default::default()
		};

		// Remove any existing container (idempotent restart).
		let rm_path = format!(
			"/v4.0.0/libpod/containers/{}?force=true",
			urlencoded(container_name)
		);
		if let Err(e) = self.client.delete_ok(&rm_path).await {
			tracing::debug!("pre-create delete {container_name}: {e}");
		}

		self.client
			.post_json::<_, serde_json::Value>("/v4.0.0/libpod/containers/create", &spec)
			.await
			.map_err(ComposeError::Podman)?;

		let start_path = format!(
			"/v4.0.0/libpod/containers/{}/start",
			urlencoded(container_name)
		);
		self.client
			.post_empty_ok(&start_path)
			.await
			.map_err(ComposeError::Podman)?;

		Ok(())
	}
}

fn build_env(service: &Service, base_dir: &Path) -> Result<Vec<String>> {
	let entries = service.env_file.to_entries();
	let env_file_vars = if !entries.is_empty() {
		env_file::load_env_file_entries(&entries, base_dir)?
	} else {
		HashMap::new()
	};
	Ok(env_file::merge_env(
		service.environment.to_map(),
		env_file_vars,
	))
}
