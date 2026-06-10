//! Container creation and start: assembles bollard `Config` from a [`Service`]
//! and starts the container. Config-building helpers live in [`super::container_config`].

use std::collections::HashMap;
use std::path::Path;

use bollard::models::{ContainerCreateBody, HostConfig, NetworkingConfig};
use bollard::query_parameters::{
	CreateContainerOptions, RemoveContainerOptions, StartContainerOptions,
};

use crate::compose::types::{ComposeFile, Service, VolumeMount, VolumeType};
use crate::error::{ComposeError, Result};
use crate::{env_file, ports, size};

use super::container_config::{
	build_healthcheck, build_log_config, build_restart_policy, build_ulimits, resolve_resources,
};
use super::container_misc::{
	build_blkio_config, build_device_requests, build_label_file_labels, opt_map, opt_vec,
	parse_device, tmpfs_options_to_string,
};
use super::network::{build_endpoint_settings, resolve_network_mode};
use super::volume_mounts::{build_binds, build_mounts};
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

		let env = build_env(service, &self.base_dir)?;

		let binds = build_binds(service, &self.base_dir);
		let secret_binds = self.build_secret_binds(service, file)?;
		let config_binds = self.build_config_binds(service, file)?;
		let all_binds: Vec<String> = binds
			.into_iter()
			.chain(secret_binds)
			.chain(config_binds)
			.collect();

		let parsed_ports = ports::parse_ports(&service.ports)?;
		let (port_bindings, exposed_ports_map) = ports::to_bollard(&parsed_ports);

		let mut exposed_port_keys: Vec<String> = exposed_ports_map.into_keys().collect();
		for raw in &service.expose {
			let key = if raw.contains('/') {
				raw.clone()
			} else {
				format!("{raw}/tcp")
			};
			if !exposed_port_keys.contains(&key) {
				exposed_port_keys.push(key);
			}
		}

		let restart_policy = build_restart_policy(service);
		let log_config = build_log_config(service.logging.as_ref());
		let (network_mode, first_network) = resolve_network_mode(service, file);
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
		for (k, v) in service.annotations.to_map() {
			labels.insert(format!("annotation.{k}"), v);
		}
		labels.insert("podup.project".to_string(), self.project.clone());
		labels.insert("podup.service".to_string(), service_name.to_string());

		let ulimits = build_ulimits(service);
		let sysctls: HashMap<String, String> = service.sysctls.to_map();
		let extra_hosts: Vec<String> = service.extra_hosts.clone();
		let dns = service.dns.to_list();
		let dns_search = service.dns_search.to_list();
		let dns_opt = service.dns_opt.to_list();

		let devices: Vec<bollard::models::DeviceMapping> = service
			.devices
			.iter()
			.map(|s| parse_device(s.as_str()))
			.collect();

		let device_requests = build_device_requests(service);

		let tmpfs_list = service.tmpfs.to_list();
		let mut tmpfs_map: HashMap<String, String> =
			tmpfs_list.into_iter().map(|p| (p, String::new())).collect();
		for v in &service.volumes {
			if let VolumeMount::Long {
				volume_type: VolumeType::Tmpfs,
				target,
				tmpfs,
				..
			} = v
			{
				let opts = tmpfs_options_to_string(tmpfs.as_ref());
				tmpfs_map.insert(target.clone(), opts);
			}
		}

		let (
			mem_limit,
			mem_reservation,
			memswap,
			nano_cpus,
			cpu_quota_eff,
			cpu_period_eff,
			pids_limit,
		) = resolve_resources(service);

		let blkio = build_blkio_config(service);

		let mut all_links: Vec<String> = service.links.clone();
		all_links.extend_from_slice(&service.external_links);

		let mounts = build_mounts(service);

		let host_config = HostConfig {
			binds: opt_vec(all_binds),
			mounts: if mounts.is_empty() {
				None
			} else {
				Some(mounts)
			},
			network_mode: network_mode.clone(),
			restart_policy,
			port_bindings: opt_map(port_bindings),
			cap_add: opt_vec(service.cap_add.clone()),
			cap_drop: opt_vec(service.cap_drop.clone()),
			sysctls: opt_map(sysctls),
			ulimits: if ulimits.is_empty() {
				None
			} else {
				Some(ulimits)
			},
			extra_hosts: opt_vec(extra_hosts),
			dns: opt_vec(dns),
			dns_search: opt_vec(dns_search),
			dns_options: opt_vec(dns_opt),
			init: service.init,
			privileged: service.privileged,
			log_config,
			pid_mode: service.pid.clone(),
			ipc_mode: service.ipc.clone(),
			uts_mode: service.uts.clone(),
			cgroup_parent: service.cgroup_parent.clone(),
			cgroupns_mode: service.cgroup.as_deref().and_then(|v| v.parse().ok()),
			shm_size: service.shm_size.as_deref().and_then(size::parse_memory),
			userns_mode: service.userns_mode.clone(),
			security_opt: opt_vec(service.security_opt.clone()),
			readonly_rootfs: service.read_only,
			devices: opt_vec(devices),
			device_cgroup_rules: opt_vec(service.device_cgroup_rules.clone()),
			tmpfs: opt_map(tmpfs_map),
			volumes_from: opt_vec(service.volumes_from.clone()),
			links: opt_vec(all_links),
			runtime: service.runtime.clone(),
			memory: mem_limit,
			memory_reservation: mem_reservation,
			memory_swap: memswap,
			memory_swappiness: service.mem_swappiness,
			nano_cpus,
			cpu_shares: service.cpu_shares.map(|s| s as i64),
			cpu_quota: cpu_quota_eff,
			cpu_period: cpu_period_eff,
			cpuset_cpus: service.cpuset.clone(),
			pids_limit,
			cpu_count: service.cpu_count,
			cpu_percent: service.cpu_percent,
			cpu_realtime_period: service.cpu_rt_period,
			cpu_realtime_runtime: service.cpu_rt_runtime,
			oom_kill_disable: service.oom_kill_disable,
			oom_score_adj: service.oom_score_adj,
			storage_opt: opt_map(service.storage_opt.clone()),
			group_add: opt_vec(service.group_add.clone()),
			blkio_weight: blkio.as_ref().and_then(|b| b.weight),
			blkio_weight_device: blkio.as_ref().and_then(|b| b.weight_device.clone()),
			blkio_device_read_bps: blkio.as_ref().and_then(|b| b.device_read_bps.clone()),
			blkio_device_write_bps: blkio.as_ref().and_then(|b| b.device_write_bps.clone()),
			blkio_device_read_iops: blkio.as_ref().and_then(|b| b.device_read_iops.clone()),
			blkio_device_write_iops: blkio.as_ref().and_then(|b| b.device_write_iops.clone()),
			device_requests: if device_requests.is_empty() {
				None
			} else {
				Some(device_requests)
			},
			annotations: opt_map(service.annotations.to_map()),
			..Default::default()
		};

		let cmd = service.command.as_ref().map(|c| c.to_exec());
		let entrypoint = service.entrypoint.as_ref().map(|c| c.to_exec());

		let networking_config = first_network.as_ref().map(|net| {
			let mut endpoints = HashMap::new();
			let svc_net_cfg = service.networks.config_for(net);
			endpoints.insert(net.clone(), build_endpoint_settings(svc_net_cfg, file));
			NetworkingConfig {
				endpoints_config: Some(endpoints),
			}
		});

		let healthcheck = service.healthcheck.as_ref().map(build_healthcheck);

		let config = ContainerCreateBody {
			image: Some(image.to_string()),
			env: opt_vec(env),
			cmd,
			entrypoint,
			host_config: Some(host_config),
			labels: opt_map(labels),
			exposed_ports: opt_vec(exposed_port_keys),
			tty: service.tty,
			open_stdin: service.stdin_open,
			user: service.user.clone(),
			working_dir: service.working_dir.clone(),
			stop_signal: service.stop_signal.clone(),
			stop_timeout: service
				.stop_grace_period
				.as_deref()
				.and_then(size::parse_duration_secs)
				.map(|s| s as i64),
			hostname: service.hostname.clone(),
			domainname: service.domainname.clone(),
			networking_config,
			healthcheck,
			..Default::default()
		};

		let _ = self
			.docker
			.remove_container(
				container_name,
				Some(RemoveContainerOptions {
					force: true,
					..Default::default()
				}),
			)
			.await;

		self.docker
			.create_container(
				Some(CreateContainerOptions {
					name: Some(container_name.to_string()),
					platform: service.platform.clone().unwrap_or_default(),
				}),
				config,
			)
			.await?;

		self.docker
			.start_container(container_name, None::<StartContainerOptions>)
			.await?;

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
