//! Container creation and start: assembles a libpod `SpecGenerator` from a
//! [`Service`] and starts the container.

use std::collections::HashMap;
use std::path::Path;

use crate::compose::types::{ComposeFile, Service};
use crate::error::{ComposeError, Result};
use crate::libpod::types::container::{LinuxResources, Namespace, SpecGenerator};
use crate::libpod::urlencoded;
use crate::libpod::API_PREFIX;
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
		// A bare `KEY` (no `=`) is a passthrough: its value comes from podup's
		// own environment, matching docker-compose. Drop it only when unset.
		let env: HashMap<String, String> = build_env(service, &self.base_dir)?
			.into_iter()
			.filter_map(|s| match s.find('=') {
				Some(idx) => Some((s[..idx].to_string(), s[idx + 1..].to_string())),
				None => std::env::var(&s).ok().map(|v| (s, v)),
			})
			.collect();

		// --- Secrets and configs become bind mounts ---
		let secret_binds = self.build_secret_binds(service, file)?;
		let config_binds = self.build_config_binds(service, file)?;
		let (mut mounts, mut named_volumes) =
			build_mounts_all(service, &self.base_dir, &secret_binds, &config_binds);
		// Resolve relative bind sources against the project base directory (and
		// expand a leading `~`) so they don't depend on Podman's working
		// directory; absolute paths (incl. staged secrets/configs) are untouched.
		for m in &mut mounts {
			if m.mount_type == "bind" {
				if let Some(src) = m.source.take() {
					m.source = Some(resolve_bind_source(&src, &self.base_dir));
				}
			}
		}
		// Map each named-volume reference to the actual volume name created by
		// create_volumes (project-prefixed, custom `name:`, or external).
		for nv in &mut named_volumes {
			nv.name = self.resolved_volume_name(&nv.name, file);
		}

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
		labels.insert("podup.config-hash".to_string(), config_hash(service));

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
			"{API_PREFIX}/containers/{}?force=true",
			urlencoded(container_name)
		);
		if let Err(e) = self.client.delete_ok(&rm_path).await {
			tracing::debug!("pre-create delete {container_name}: {e}");
		}

		self.client
			.post_json::<_, serde_json::Value>(&format!("{API_PREFIX}/containers/create"), &spec)
			.await
			.map_err(ComposeError::Podman)?;

		let start_path = format!(
			"{API_PREFIX}/containers/{}/start",
			urlencoded(container_name)
		);
		self.client
			.post_empty_ok(&start_path)
			.await
			.map_err(ComposeError::Podman)?;

		Ok(())
	}

	/// Resolve a service's named-volume reference to the actual volume name
	/// that `create_volumes` produced: a custom `name:`, the raw name for an
	/// external volume, or the `{project}_{name}` form. References not declared
	/// in the top-level `volumes:` map (anonymous/implicit) are left unchanged.
	fn resolved_volume_name(&self, reference: &str, file: &ComposeFile) -> String {
		resolve_volume_name(reference, &self.project, file)
	}
}

/// Resolve a named-volume reference to the volume name `create_volumes`
/// produced: a custom `name:`, the raw name for an external volume, or the
/// `{project}_{name}` form. References not declared in the top-level `volumes:`
/// map (anonymous/implicit) are returned unchanged.
fn resolve_volume_name(reference: &str, project: &str, file: &ComposeFile) -> String {
	match file.volumes.get(reference) {
		Some(cfg) => {
			if let Some(name) = cfg.as_ref().and_then(|c| c.name.as_deref()) {
				name.to_string()
			} else if cfg.as_ref().and_then(|c| c.external).unwrap_or(false) {
				reference.to_string()
			} else {
				format!("{project}_{reference}")
			}
		}
		None => reference.to_string(),
	}
}

/// Resolve a bind-mount source path: expand a leading `~`, then make a relative
/// path absolute against the project base directory. Absolute paths (including
/// staged secret/config files) are returned unchanged.
fn resolve_bind_source(src: &str, base_dir: &Path) -> String {
	if src.is_empty() {
		return src.to_string();
	}
	let expanded = if let Some(rest) = src.strip_prefix("~/") {
		match std::env::var("HOME") {
			Ok(home) => format!("{home}/{rest}"),
			Err(_) => src.to_string(),
		}
	} else if src == "~" {
		std::env::var("HOME").unwrap_or_else(|_| src.to_string())
	} else {
		src.to_string()
	};
	if Path::new(&expanded).is_absolute() {
		expanded
	} else {
		base_dir.join(&expanded).to_string_lossy().into_owned()
	}
}

/// Stable content hash of a service definition, stored as the
/// `podup.config-hash` label. On `up`, comparing this against the label on an
/// existing container tells podup whether the service configuration changed
/// and the container must be recreated, or is unchanged and can be left as is.
pub(crate) fn config_hash(service: &Service) -> String {
	use sha2::{Digest, Sha256};
	// Canonicalise through `serde_json::Value` first: object keys are emitted in
	// sorted order, so map-typed fields (e.g. `storage_opt`) cannot reorder
	// between runs and flap the hash into a spurious recreate.
	let serialized = serde_json::to_value(service)
		.and_then(|v| serde_json::to_vec(&v))
		.unwrap_or_default();
	Sha256::digest(&serialized)
		.iter()
		.map(|b| format!("{b:02x}"))
		.collect()
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

#[cfg(test)]
mod tests {
	use super::{config_hash, resolve_volume_name};
	use crate::parse_str;

	#[test]
	#[cfg(unix)]
	fn bind_source_resolution() {
		use super::resolve_bind_source;
		use std::path::Path;
		let base = Path::new("/srv/app");
		assert_eq!(resolve_bind_source("/abs/path", base), "/abs/path");
		assert_eq!(resolve_bind_source("./data", base), "/srv/app/./data");
		assert_eq!(resolve_bind_source("data", base), "/srv/app/data");
		std::env::set_var("HOME", "/home/u");
		assert_eq!(resolve_bind_source("~/x", base), "/home/u/x");
		assert_eq!(resolve_bind_source("~", base), "/home/u");
	}

	#[test]
	fn volume_name_resolution() {
		let f = parse_str(
			"services:\n  s:\n    image: x\nvolumes:\n  data:\n  ext:\n    external: true\n  custom:\n    name: my-vol\n",
		)
		.unwrap();
		assert_eq!(resolve_volume_name("data", "proj", &f), "proj_data");
		assert_eq!(resolve_volume_name("ext", "proj", &f), "ext");
		assert_eq!(resolve_volume_name("custom", "proj", &f), "my-vol");
		// Not declared in top-level volumes -> left as-is.
		assert_eq!(resolve_volume_name("anon", "proj", &f), "anon");
	}

	#[test]
	fn config_hash_is_stable_and_sensitive() {
		let a = parse_str("services:\n  web:\n    image: nginx:1.27\n").unwrap();
		let b = parse_str("services:\n  web:\n    image: nginx:1.27\n").unwrap();
		let c = parse_str("services:\n  web:\n    image: nginx:1.28\n").unwrap();
		let ha = config_hash(&a.services["web"]);
		let hb = config_hash(&b.services["web"]);
		let hc = config_hash(&c.services["web"]);
		assert_eq!(ha, hb, "same config produces the same hash");
		assert_ne!(ha, hc, "a changed image produces a different hash");
		assert_eq!(ha.len(), 64, "sha-256 hex is 64 chars");
	}

	#[test]
	fn config_hash_stable_despite_map_field_order() {
		// `storage_opt` is a HashMap; canonical serialisation must sort its keys
		// so the hash does not flap and trigger a spurious recreate on `up`.
		let a = parse_str(
			"services:\n  web:\n    image: x\n    storage_opt:\n      size: \"10G\"\n      foo: bar\n      baz: qux\n",
		)
		.unwrap();
		let b = parse_str(
			"services:\n  web:\n    image: x\n    storage_opt:\n      baz: qux\n      size: \"10G\"\n      foo: bar\n",
		)
		.unwrap();
		assert_eq!(
			config_hash(&a.services["web"]),
			config_hash(&b.services["web"]),
			"hash must be independent of storage_opt key order",
		);
	}
}
