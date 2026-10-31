//! Container creation and start: assembles a libpod `SpecGenerator` from a
//! [`Service`] and starts the container.

use std::collections::HashMap;

use crate::compose::types::{ComposeFile, Service};
use crate::error::{ComposeError, Result};
use crate::libpod::types::container::{LinuxResources, Namespace};
use crate::libpod::urlencoded;
use crate::libpod::validate::pre_validate_spec;
use crate::libpod::API_PREFIX;
use crate::size;

mod anon_volumes;
mod fields;
mod inputs;
mod oneoff;
mod resolve;
mod security;
mod spec;
mod warnings;

pub(crate) use resolve::{config_hash, effective_env, resolve_bind_source};
use resolve::{resolve_links, resolve_stop_signal, resolve_volumes_from};
pub(crate) use spec::{build_spec_generator, NamespaceInputs, SecurityInputs, SpecInputs};
use warnings::host_mode;
use warnings::rootless;

pub(crate) use fields::device_spec_parts;
pub(crate) use host_mode::check_host_mode;
pub(crate) use security::parse_device_cgroup_rule;

use crate::engine::container_config::{
	build_healthcheck, build_log_config, build_resource_limits, build_restart_policy, build_ulimits,
};
use crate::engine::network::resolve_network_mode;
use crate::engine::volume_mounts::build_mounts_all;
use crate::engine::Engine;
use fields::{build_blkio_config, warn_swarm_only_deploy};
// Re-exported at the container root so `crate::engine::container::*` is a
// valid path inside the crate (lib.rs's `effective_no_new_privileges`
// wrapper reaches it for the audit module, #1743). Private here would shadow
// the `pub(crate)` visibility into private.
pub(crate) use security::parse_bool_value;
pub(crate) use security::parse_security_opts;

impl Engine {
	pub(in crate::backends::engine) async fn create_and_start(
		&self,
		container_name: &str,
		service_name: &str,
		service: &Service,
		file: &ComposeFile,
		start: bool,
		env: &std::collections::BTreeMap<String, String>,
	) -> Result<()> {
		self.create_container(container_name, service_name, service, file, env)
			.await?;
		if start {
			let start_path = format!(
				"{API_PREFIX}/containers/{}/start",
				urlencoded(container_name)
			);
			self.client
				.post_empty_ok(&start_path)
				.await
				.map_err(ComposeError::Podman)?;
		}
		Ok(())
	}

	/// Create a container without starting it, retaining the daemon's response
	/// so a one-off run can clean up its own creation by ID after a failed start.
	pub(in crate::backends::engine) async fn create_container(
		&self,
		container_name: &str,
		service_name: &str,
		service: &Service,
		file: &ComposeFile,
		env: &std::collections::BTreeMap<String, String>,
	) -> Result<serde_json::Value> {
		// Reject an invalid container name client-side (podman would otherwise
		// answer with an opaque HTTP 500 from its name-regex check). Covers the
		// ad-hoc `run --name` path too, which never passes through the up-time
		// preflight in `validate_object_names`.
		crate::engine::project::names::ensure_valid_object_name(
			"container",
			service_name,
			container_name,
		)?;

		let derived_image;
		let image: &str = if let Some(img) = service.image.as_deref() {
			img
		} else if let Some(build) = service.build.as_ref() {
			// No `image:`, so reference the exact tag the build step produced for this
			// build-only service (project-scoped `{project}-{service}:latest`, or
			// the first `build.tags` entry). Must stay in lockstep with
			// `primary_build_tag`, or `up --build` creates the container against a
			// name no built image carries (HTTP 404: no such image).
			derived_image = crate::engine::build::primary_build_tag(
				&self.project,
				service_name,
				None,
				build.tags(),
			);
			&derived_image
		} else {
			return Err(ComposeError::NoImageOrBuild(service_name.into()));
		};

		warn_swarm_only_deploy(service_name, service);

		// --- Environment ---
		// The environment the container runs with, resolved by the caller: the
		// same map the config hash is built from, so the label this container
		// carries and the environment it is created with describe one snapshot.
		let spec_env: HashMap<String, String> =
			env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

		// Every secret/config source (inline, `file:` and `external: true`) is
		// injected as a Podman-native secret, never a bind mount. The ones podup
		// owns are created up front by `create_project_secrets`; here we only build
		// the references and preflight external ones for existence.
		let native_secrets = self.build_native_secrets(service, file).await?;
		let (mut mounts, mut named_volumes) = build_mounts_all(service, &self.base_dir);
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
			nv.name = self.resolved_volume_name(&nv.name, file)?;
		}
		// The container this one replaces may carry anonymous volumes: one
		// per compose `volumes:` entry declared without a source, and one per
		// `VOLUME` directive of the image. The replacement inherits them by
		// destination, so the data they hold survives the recreate instead
		// of staying behind in a volume nothing points at. `-V/--renew-anon-
		// volumes` is the way to ask for fresh ones instead.
		if !self.renew_anon_volumes {
			self.inherit_anonymous_volumes(container_name, image, &mut named_volumes, &mounts)
				.await?;
		}

		// --- Port mappings ---
		// In pod mode the pod publishes every service's ports: a container
		// inside a pod cannot publish ports of its own, so the per-container
		// list is empty. The pod already aggregated the union via
		// `Engine::ensure_pod`; sending ports here would conflict.
		let in_pod = file
			.podman_pod()
			.map_err(crate::error::ComposeError::Unsupported)?;
		let (portmappings, expose) = self.compute_portmappings_and_expose(service, in_pod)?;

		// --- Restart policy ---
		let (restart_policy, restart_tries) = build_restart_policy(service);

		// --- Logging ---
		let log_configuration = build_log_config(service_name, service.logging.as_ref())?;

		// --- Networks ---
		// In pod mode the container joins the pod's shared namespace, so its
		// own `networks:` map and `netns` are empty. Pod mode rejects any
		// service carrying `network_mode:` up front (see
		// `validate_pod_or_refuse`), so the only branch here is "no
		// per-container networks".
		let (netns, mut networks) = if in_pod {
			(None, HashMap::new())
		} else {
			resolve_network_mode(service_name, service, file, &self.project)
		};
		oneoff::drop_service_aliases(service, &mut networks);

		// --- Labels ---
		let labels = self.compute_container_labels(service, file, service_name, env)?;

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
		let (devices, device_cgroup_rule) = self.compute_devices_and_rules(service);

		// --- Security options (decomposed onto SpecGenerator fields) ---
		let security = parse_security_opts(service);

		// Pre-validate the `SpecGenerator` fields libpod validates on its own
		// (namespace modes, `device_cgroup_rule` access strings), so a rejected
		// value surfaces as a `PodmanError::Field` carrying the compose-side
		// field name and offending value instead of libpod's raw validator
		// text. The up-front `run_up` call has already rejected any invalid
		// value before a single resource was created (#1867); this per-service
		// call is a belt to that loop's braces, kept here so a future field
		// added to the validator that only `create_and_start` sees still
		// surfaces as the same field-shaped error.
		pre_validate_spec(service_name, service)?;

		// --- Namespace modes, platform, links, volumes_from ---
		let pidns = service.pid.as_deref().map(Namespace::parse);
		let ipcns = service.ipc.as_deref().map(Namespace::parse);
		let utsns = service.uts.as_deref().map(Namespace::parse);
		let cgroupns = service.cgroup.as_deref().map(Namespace::parse);
		// In pod mode the user namespace is the pod's; a member must not set
		// its own (Podman's CLI refuses the pair, and the API would give the
		// member a namespace the pod does not share).
		let userns = if in_pod {
			None
		} else {
			service.userns_mode.as_deref().map(Namespace::parse)
		};
		// Read before `userns` moves into the spec. Both `keep-id` and
		// `keep-id:<opts>` parse to the `keep-id` mode; a pod member has no
		// user namespace of its own and never takes the lock.
		let keep_id_create = userns.as_ref().is_some_and(|ns| ns.mode() == "keep-id");
		let (image_os, image_arch) = service
			.platform
			.as_deref()
			.and_then(|p| p.split_once('/'))
			.map(|(os, arch)| (Some(os.to_string()), Some(arch.to_string())))
			.unwrap_or((None, None));
		let links = resolve_links(service, file, &self.project);
		let volumes_from = resolve_volumes_from(service, file, &self.project);
		let stop_timeout = service
			.stop_grace_period
			.as_deref()
			.and_then(size::parse_duration_secs_rounded_up);

		if service.mac_address.is_some() {
			tracing::warn!(
				"service \"{service_name}\": top-level mac_address is deprecated; \
				move it to networks.<network>.mac_address"
			);
		}

		for warning in rootless::rootless_caveat_warnings(service_name, service) {
			tracing::warn!("{warning}");
		}

		// Surface every active host-binding / privilege-escalation mode the
		// compose file declared. The warning is emitted *before* the spec is
		// POSTed so a host-mode the operator did not intend never reaches the
		// daemon: the log line is the only signal they get, and it has to
		// arrive before the API call succeeds. `--no-warn` is the escape
		// hatch for operators who wrote the compose file deliberately.
		if !self.no_warn {
			for w in check_host_mode(service_name, service) {
				tracing::warn!("{}", w.message);
			}
		}

		let stop_signal = service
			.stop_signal
			.as_deref()
			.map(resolve_stop_signal)
			.transpose()?;

		let spec = build_spec_generator(
			&self.project,
			service,
			service.healthcheck.as_ref().and_then(build_healthcheck),
			log_configuration,
			match service
				.healthcheck
				.as_ref()
				.map(crate::compose::types::HealthCheck::podman_on_failure)
				.transpose()
			{
				Ok(action) => action.flatten().map(|a| match a {
					crate::compose::types::HealthOnFailure::None => {
						crate::libpod::types::container::HealthCheckOnFailureAction::None
					}
					crate::compose::types::HealthOnFailure::Kill => {
						crate::libpod::types::container::HealthCheckOnFailureAction::Kill
					}
					crate::compose::types::HealthOnFailure::Restart => {
						crate::libpod::types::container::HealthCheckOnFailureAction::Restart
					}
					crate::compose::types::HealthOnFailure::Stop => {
						crate::libpod::types::container::HealthCheckOnFailureAction::Stop
					}
				}),
				Err(e) => {
					return Err(crate::error::ComposeError::Unsupported(format!(
						"{service_name}: {e}"
					)))
				}
			},
			SpecInputs {
				in_pod,
				container_name: container_name.to_string(),
				image: image.to_string(),
				portmappings,
				expose,
				networks,
				netns,
				labels,
				annotations,
				sysctl,
				resource_limits,
				ulimits,
				mounts,
				named_volumes,
				volumes_from,
				native_secrets,
				devices,
				device_cgroup_rule,
				security: SecurityInputs {
					selinux_opts: security.selinux_opts,
					apparmor_profile: security.apparmor_profile,
					seccomp_profile_path: security.seccomp_profile_path,
					no_new_privileges: security.no_new_privileges,
					mask: security.mask,
					unmask: security.unmask,
				},
				namespaces: NamespaceInputs {
					userns,
					pidns,
					ipcns,
					utsns,
					cgroupns,
				},
				restart: (restart_policy, restart_tries),
				stop_signal_timeout: (stop_signal, stop_timeout),
				command: service.command.as_ref().map(|c| c.to_exec()),
				entrypoint: service.entrypoint.as_ref().map(|c| c.to_exec()),
				env: spec_env,
				links,
				image_platform: (image_os, image_arch),
				storage_opts: service.storage_opt.clone(),
			},
		)?;

		// Remove this project's existing container (idempotent restart). `up
		// -V/--renew-anon-volumes` also drops its old anonymous volumes (v=true)
		// so they are recreated fresh instead of orphaned. Only a container
		// carrying this project's label is replaced: one that is absent needs no
		// delete, and one that is not ours is refused rather than force-removed
		// with whatever it wrote to its layer.
		let owner = self
			.object_owner("containers", container_name, &["/Config/Labels"])
			.await?;
		self.require_owned("container", container_name, &owner)?;
		if owner != crate::backends::engine::objects::ownership::Owner::Absent {
			let rm_path = format!(
				"{API_PREFIX}/containers/{}?force=true&v={}",
				urlencoded(container_name),
				self.renew_anon_volumes,
			);
			if let Err(e) = self.client.delete_ok(&rm_path).await {
				tracing::debug!("pre-create delete {container_name}: {e}");
			}
		}

		// Only the create is serialised (`Engine::keep_id_create`); the guard
		// drops before the start, so starts stay concurrent.
		let keep_id_guard = if keep_id_create {
			Some(self.keep_id_create.lock().await)
		} else {
			None
		};
		let created = self
			.client
			.post_json::<_, serde_json::Value>(&format!("{API_PREFIX}/containers/create"), &spec)
			.await
			.map_err(ComposeError::Podman)?;
		drop(keep_id_guard);

		Ok(created)
	}
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

// The fake Podman speaks over a Unix socket, so this test does not build
// on Windows, like every other test that starts it.
#[cfg(all(test, unix))]
mod spec_body_tests;

#[cfg(all(test, unix))]
#[path = "anon_volumes_tests.rs"]
mod anon_volumes_tests;
