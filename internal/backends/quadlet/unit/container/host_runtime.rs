//! Host resources and execution settings that select what the container runs.

use std::collections::HashSet;

use crate::compose::types::Service;

use super::{quote_podman_arg_value, Section};

/// Emit one `AddDevice=` line per CDI device requested by the service.
///
/// A device already named by `devices:` is not repeated: Podman rejects a
/// duplicate device on one container, and both spellings reach the same
/// `--device` argument.
pub(super) fn emit_cdi_devices(service: &Service, container: &mut Section) {
	let mut seen: HashSet<String> = service.devices.iter().cloned().collect();
	for device in crate::engine::cdi_devices(service) {
		if seen.insert(device.clone()) {
			container.add("AddDevice", device);
		}
	}
}

/// Emit the image platform and OCI runtime selected by the service.
///
/// The complete platform value reaches `podman run`, so its optional variant
/// survives. The runtime is a global Podman option and therefore belongs in
/// `GlobalArgs=`, before the `run` subcommand.
pub(super) fn emit_platform_and_runtime(service: &Service, container: &mut Section) {
	if let Some(platform) = service
		.platform
		.as_deref()
		.map(str::trim)
		.filter(|platform| !platform.is_empty())
	{
		container.add(
			"PodmanArgs",
			format!("--platform={}", quote_podman_arg_value(platform)),
		);
	}
	if let Some(runtime) = service
		.runtime
		.as_deref()
		.map(str::trim)
		.filter(|runtime| !runtime.is_empty())
	{
		container.add(
			"GlobalArgs",
			format!("--runtime={}", quote_podman_arg_value(runtime)),
		);
	}
}
