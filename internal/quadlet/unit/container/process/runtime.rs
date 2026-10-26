//! How the process inside the container is set up: its user, its working
//! directory, and the flags that shape the run.

use crate::compose::types::Service;
use crate::quadlet::is_no_warn_set;

use super::super::super::pod_fields::{PodField, PodFields};
use super::super::super::shared_values::hostname;
use super::super::Section;

/// Emit the process-level settings of the container: `HostName=`, `User=`/
/// `Group=`, `WorkingDir=`, `ReadOnly=`, `RunInit=`, and the `privileged: true`
/// escape hatch.
pub(in super::super) fn emit_runtime(
	name: &str,
	service: &Service,
	in_pod: bool,
	pod_fields: &PodFields,
	container: &mut Section,
) {
	if !(in_pod && pod_fields.owns(PodField::HostName)) {
		if let Some(hostname) = hostname(service) {
			container.add("HostName", hostname.to_string());
		}
	}
	if let Some(user) = &service.user {
		// Quadlet `User=` takes a UID/username only; a `uid:gid` compose value
		// must be split so the GID lands in the dedicated `Group=` key (Quadlet
		// recombines them into `--user uid:gid`).
		match user.split_once(':') {
			Some((uid, gid)) => {
				container.add("User", uid.to_string());
				container.add("Group", gid.to_string());
			}
			None => container.add("User", user.clone()),
		}
	}
	if let Some(text) = service.umask.as_deref().filter(|text| {
		crate::compose::types::service::service_umask::validate_runtime(name, text).is_ok()
	}) {
		let mask = crate::compose::types::service::service_umask::normalized(text);
		// A CLI flag keeps the unit readable by Quadlet at the Podman 5.0 floor.
		container.add("PodmanArgs", format!("--umask={mask}"));
	}
	if let Some(wd) = &service.working_dir {
		container.add("WorkingDir", wd.clone());
	}
	if service.read_only == Some(true) {
		container.add("ReadOnly", "true".to_string());
	}
	if service.privileged == Some(true) {
		// No dedicated [Container] key exists for privileged mode; pass it through
		// as a raw podman flag, like the other escape-hatch fields.
		container.add("PodmanArgs", "--privileged".to_string());
		// The live `up` engine emits a per-call warning for every active
		// host-binding mode. Quadlet cannot call the engine (it has no Podman
		// client), so it surfaces the same warning here, at generate time, so
		// the operator sees the warning whichever path they use. The mode is
		// still emitted below; the warning is the point. `--no-warn` opts
		// the operator out of this one too (set via the `NoWarnGuard` the CLI
		// driver wraps around `write_quadlet`).
		if !is_no_warn_set() {
			tracing::warn!(
				"service \"{name}\": privileged: true grants every Linux capability and exposes \
				 every host device; under rootless Podman the effect is reduced but the container \
				 still bypasses the default capability set"
			);
		}
	}
	if service.init == Some(true) {
		container.add("RunInit", "true".to_string());
	}
}
