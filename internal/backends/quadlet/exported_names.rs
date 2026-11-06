//! Names from a compose file that Quadlet would read as something else.
//!
//! The image goes to `Image=`, a network to `Network=` and a volume to
//! `Volume=`, and Quadlet gives each a meaning beyond the string: a value that
//! starts with `-` takes the place of the image in the `podman run` line and is
//! read as a flag, `x.network` and `x.container` name a unit, `container:other`
//! and `ns:/path` join another namespace, `x.volume` names a unit. `up` reads
//! the same names as what they say and fails to find or pull them, so the unit
//! has to refuse them as well rather than write a line that means more than the
//! compose file did.

use crate::compose::types::{ComposeFile, VolumeMount};
use crate::error::{ComposeError, Result};

/// Whether `name` is an ordinary podman object name: a leading alphanumeric,
/// then alphanumerics, `_`, `.` or `-`. A `:` or `/` is what makes `Network=`
/// and `Volume=` read the value as a mode, a path or a `source:target` pair.
fn is_plain_name(name: &str) -> bool {
	crate::libpod::is_valid_object_name(name)
}

fn refuse(kind: &str, key: &str, name: &str, why: &str) -> ComposeError {
	let origin = if key == name {
		String::new()
	} else {
		format!(" (from {kind} '{key}')")
	};
	ComposeError::Unsupported(format!(
		"external {kind} '{name}'{origin} cannot be written in a Quadlet unit: {why}"
	))
}

/// The volume keys a service mounts by name.
fn mounted_volume_keys(file: &ComposeFile) -> Vec<&str> {
	let mut keys = Vec::new();
	for service in file.services.values() {
		for mount in &service.volumes {
			match mount {
				VolumeMount::Short(text) => keys.extend(text.split(':').next()),
				VolumeMount::Long { source, .. } => keys.extend(source.as_deref()),
			}
		}
	}
	keys
}

/// Refuse, for every service and every external network or volume a service
/// uses, a value Quadlet would not read as the plain name it is.
pub(super) fn validate_exported_names(file: &ComposeFile) -> Result<()> {
	for (service_name, service) in &file.services {
		if let Some(image) = service.image.as_deref() {
			if image.starts_with('-') {
				return Err(ComposeError::Unsupported(format!(
					"service '{service_name}': image '{image}' starts with '-', which `podman run` would read as an option"
				)));
			}
		}
	}
	let used_networks: Vec<String> = file
		.services
		.values()
		.flat_map(|service| service.networks.names())
		.collect();
	for (key, config) in &file.networks {
		let Some(config) = config.as_ref().filter(|c| c.external == Some(true)) else {
			continue;
		};
		if !used_networks.iter().any(|used| used == key) {
			continue;
		}
		let name = config.name.as_deref().unwrap_or(key.as_str());
		if !is_plain_name(name) {
			return Err(refuse(
				"network",
				key,
				name,
				"`Network=` reads a name with `:` or `/` as a namespace mode",
			));
		}
		if name.ends_with(".network") || name.ends_with(".container") {
			return Err(refuse(
				"network",
				key,
				name,
				"`Network=` reads a name ending in `.network` or `.container` as a Quadlet unit",
			));
		}
	}
	let used_volumes = mounted_volume_keys(file);
	for (key, config) in &file.volumes {
		let Some(config) = config.as_ref().filter(|c| c.external == Some(true)) else {
			continue;
		};
		if !used_volumes.contains(&key.as_str()) {
			continue;
		}
		let name = config.name.as_deref().unwrap_or(key.as_str());
		if !is_plain_name(name) {
			return Err(refuse(
				"volume",
				key,
				name,
				"`Volume=` reads a `:` or `/` in the name as part of the mount",
			));
		}
		if name.ends_with(".volume") {
			return Err(refuse(
				"volume",
				key,
				name,
				"`Volume=` reads a name ending in `.volume` as a Quadlet unit",
			));
		}
	}
	Ok(())
}
