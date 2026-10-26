//! Build the `.container` unit for a service.

use indexmap::IndexMap;

use crate::compose::dependencies::effective_depends_on;
use crate::compose::types::{SecretConfig, Service};

use super::health::render_healthcheck;
use super::security::{is_inline_secret, map_security_opt, render_secret};
// Shared helpers from the sibling `render`/`warnings`/`security` modules and
// the parent `QuadletUnit` type, imported here so the emitters take them from
// `super::` rather than reaching past this module, as the sibling `unit`
// submodules already do.
use super::pod::PodFields;
use super::{
	abs_against, collect_warnings, owner_marker, quote_podman_arg_value, render_command,
	render_publish_port, render_restart, render_tmpfs_mount, render_volume, safe_unit_stem,
	sorted_label_pairs, sorted_pairs, unit_stem, QuadletUnit, Section,
};

mod confinement;
mod definition;
mod networking;
mod process;
mod storage;

pub(crate) use definition::container_name;
pub(crate) use networking::is_rootless_user_mode;

use confinement::{
	emit_capabilities, emit_deploy_limits, emit_group_add, emit_limits, emit_security_opts,
	emit_ulimits, emit_userns,
};
use definition::{
	emit_annotations, emit_identity, emit_image, emit_labels, emit_pull, emit_sysctls,
};
use networking::{
	emit_expose, emit_host_config, emit_network_extras, emit_network_mode, emit_networks,
	emit_ports,
};
use process::{
	emit_command, emit_env_files, emit_environment, emit_logging, emit_restart, emit_runtime,
	emit_stop,
};
use storage::{emit_secrets, emit_tmpfs, emit_volumes};

/// Project-wide inputs every generated unit needs, as opposed to the per-service
/// ones (`name`, `service`). Grouped rather than passed loose because the set
/// only grows as more compose keys gain a Quadlet mapping, and because they are
/// identical for every service in one `generate_at` call.
pub(crate) struct UnitContext<'a> {
	/// Compose project name, stamped as the `podup.project` ownership label.
	pub project: &'a str,
	/// Volumes the compose file declares (external ones excluded).
	pub declared_volumes: &'a [&'a str],
	/// Networks the compose file declares (external ones excluded).
	pub declared_networks: &'a [&'a str],
	/// Top-level `secrets:` definitions, for resolving a service's secret refs.
	pub secrets: &'a IndexMap<String, SecretConfig>,
	/// Directory compose resolves relative paths against: the compose file's
	/// own directory, not the unit's. See [`abs_against`].
	pub base_dir: &'a std::path::Path,
	/// Every service in the file, so a `depends_on` condition can be judged
	/// against the service it names rather than in the abstract.
	pub services: &'a IndexMap<String, Service>,
	/// `true` when the compose file opts into `x-podman-pod: true`. Each
	/// container unit then references the project pod by `Pod=<stem>.pod`
	/// and drops its own `PublishPort=` and `Network=` lines.
	pub pod_mode: bool,
	/// The `network_mode` the pod carries, when `pod_mode` is true and
	/// every service agreed on a pasta/slirp4netns mode. `None` when the
	/// project did not agree on one (the pod runs on the project
	/// networks, or has no `Network=` line at all). Read by the per-service
	/// `Network=` rule to decide whether a per-service mode is the one the
	/// pod carries (no warning) or something else (warning, the same one
	/// the live `up` path emits when the project would be refused).
	pub pod_network_mode: Option<&'a str>,
	/// Fields agreed by every service for the project pod. A container drops a
	/// shared value only when the pod carries it.
	pub pod_fields: &'a PodFields,
}

/// Which `.container` shape to render. The default (`Standard`) is what
/// `podup generate quadlet` always emits: a buildable service points its
/// `Image=` at the sibling `.build` unit so Quadlet runs the build every
/// time the container starts. `Prebuilt` is for quadlet-mode autostart only:
/// the image is built once at install, the container unit references the
/// concrete tag (the `.build` unit's `ImageTag=`) instead of the `.build`
/// filename, and `Pull=never` stops Quadlet from racing the local build
/// against a registry pull. Services without a buildable `build:` render
/// identically under both modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerUnitMode {
	/// `generate quadlet` output: `Image=<stem>.build` for a buildable service,
	/// which makes Quadlet add `Requires=`/`After=` on the build service.
	Standard,
	/// Autostart prebuilt output: `Image=<tag>` (no `.build` reference) and
	/// `Pull=never`; Quadlet then adds no dependency on the build service.
	Prebuilt,
}

/// Build the `.container` unit for one compose `service`.
///
/// The project name is stamped onto the unit as the `podup.project` ownership
/// label (and the service key as `podup.service`), matching the labels the live
/// engine applies, so generated containers are traceable back to their project
/// the same way running ones are. The [`ContainerUnitMode`] selects between
/// the `Image=<stem>.build` shape `podup generate quadlet` always emits, and
/// the `Image=<tag>` + `Pull=never` shape quadlet-mode autostart needs.
///
/// The `[Container]` section is written by the `emit_*` helpers below, one per
/// compose concern, each called in the order their keys appear in the section.
pub(crate) fn container_unit_with_mode(
	name: &str,
	service: &Service,
	ctx: &UnitContext<'_>,
	warnings: &mut Vec<String>,
	mode: ContainerUnitMode,
) -> QuadletUnit {
	render_container_unit(name, service, ctx, warnings, mode, UnitRole::Service)
}

/// Build the `.container` unit for a scheduled job.
///
/// It goes through the same emitters as a service, so a job's `command`,
/// `environment`, volumes and limits are written by the code that writes them
/// for a service. Only the lifecycle differs: the unit is `Type=oneshot`, has
/// no restart policy, and has no `[Install]` section, because a systemd timer
/// starts it and nothing enables it.
pub(crate) fn job_container_unit(
	name: &str,
	service: &Service,
	ctx: &UnitContext<'_>,
	warnings: &mut Vec<String>,
) -> QuadletUnit {
	render_container_unit(
		name,
		service,
		ctx,
		warnings,
		ContainerUnitMode::Standard,
		UnitRole::Job,
	)
}

/// What the `.container` unit stands for, which decides its `[Service]` and
/// `[Install]` sections.
#[derive(Clone, Copy, PartialEq, Eq)]
enum UnitRole {
	/// A compose service: restart policy, enabled at boot.
	Service,
	/// A scheduled job: one run per start, started by a timer.
	Job,
}

fn render_container_unit(
	name: &str,
	service: &Service,
	ctx: &UnitContext<'_>,
	warnings: &mut Vec<String>,
	mode: ContainerUnitMode,
	role: UnitRole,
) -> QuadletUnit {
	let UnitContext {
		project,
		declared_volumes,
		declared_networks,
		secrets,
		base_dir,
		services,
		pod_mode,
		pod_network_mode,
		pod_fields,
	} = ctx;
	let in_pod: bool = *pod_mode;
	let mut unit = Section::new("Unit");
	let kind = if role == UnitRole::Job {
		"podup job"
	} else {
		"podup"
	};
	unit.add("Description", format!("{name} ({kind})"));
	let deps = effective_depends_on(service, services);
	for dep in deps.service_names() {
		// The dependency's generated unit is named `{unit_stem(project, dep)}.container`,
		// so its service is `{unit_stem(project, dep)}.service`; reference that, not the
		// raw compose key, or the ordering would target a non-existent unit.
		let dep_service = format!("{}.service", unit_stem(project, &dep));
		unit.add("After", dep_service.clone());
		if deps.required_for(&dep) {
			unit.add("Requires", dep_service);
		} else {
			unit.add("Wants", dep_service);
		}
	}

	let mut container = Section::new("Container");
	emit_identity(project, name, service, in_pod, &mut container);
	emit_image(project, name, service, mode, &mut container);
	emit_runtime(name, service, in_pod, pod_fields, &mut container);
	emit_ports(service, in_pod, &mut container);
	emit_environment(service, &mut container);
	emit_volumes(service, project, declared_volumes, &mut container);
	emit_networks(service, project, declared_networks, in_pod, &mut container);
	emit_labels(name, project, service, &mut container, warnings);
	emit_capabilities(service, &mut container);
	emit_command(service, &mut container);
	emit_annotations(service, &mut container);
	emit_env_files(service, base_dir, &mut container);
	emit_tmpfs(service, &mut container);
	emit_sysctls(service, &mut container);
	emit_ulimits(service, &mut container);
	emit_host_config(service, in_pod, pod_fields, &mut container);
	emit_limits(service, in_pod, pod_fields, &mut container);
	emit_userns(service, in_pod, pod_fields, &mut container);
	emit_stop(service, &mut container);
	emit_network_mode(
		name,
		project,
		service,
		in_pod,
		*pod_network_mode,
		&mut container,
	);
	emit_group_add(service, &mut container);
	emit_expose(service, &mut container);
	emit_network_extras(service, in_pod, pod_fields, &mut container);
	emit_security_opts(service, name, &mut container, warnings);
	emit_logging(name, service, &mut container, warnings);
	emit_pull(service, mode, &mut container);
	emit_deploy_limits(service, &mut container);
	emit_secrets(name, project, service, secrets, &mut container, warnings);
	render_healthcheck(name, service, &mut container, warnings);

	let mut svc = Section::new("Service");
	match role {
		UnitRole::Service => emit_restart(service, &mut svc),
		// One run per start. A scheduled run is not retried, and `Restart=`
		// is only valid next to `oneshot` as `no` or `on-failure`.
		UnitRole::Job => svc.add("Type", "oneshot".to_string()),
	}

	collect_warnings(
		name,
		service,
		services,
		warnings,
		mode == ContainerUnitMode::Prebuilt && super::build::emits_build_unit(service),
	);

	// The unforgeable ownership marker comes first, as its own comment line;
	// see `owner_marker` for why it must stay separate from the `Label=` line.
	let mut contents = owner_marker(project);
	contents.push_str(&unit.render());
	contents.push('\n');
	contents.push_str(&container.render());
	if !svc.is_empty() {
		contents.push('\n');
		contents.push_str(&svc.render());
	}
	if role == UnitRole::Service {
		contents.push_str("\n[Install]\nWantedBy=default.target\n");
	}

	QuadletUnit {
		filename: format!("{}.container", unit_stem(project, name)),
		contents,
	}
}
