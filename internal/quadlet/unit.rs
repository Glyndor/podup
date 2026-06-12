//! Build the individual `.network`, `.volume` and `.container` units.

use crate::compose::types::{PortMapping, Service};
use crate::ports::parse_ports;

use super::render::{
	render_command, render_publish_port, render_restart, render_volume, safe_unit_stem,
	sanitize_value, sorted_label_pairs, sorted_pairs, Section,
};
use super::warnings::collect_warnings;
use super::QuadletUnit;

pub(super) fn network_unit(name: &str, project: &str, _has_config: bool) -> QuadletUnit {
	let value = sanitize_value(&format!("{project}_{name}"));
	let contents =
		format!("[Network]\nNetworkName={value}\n\n[Install]\nWantedBy=default.target\n");
	QuadletUnit {
		filename: format!("{}.network", safe_unit_stem(name)),
		contents,
	}
}

pub(super) fn volume_unit(name: &str, project: &str, _has_config: bool) -> QuadletUnit {
	let value = sanitize_value(&format!("{project}_{name}"));
	let contents = format!("[Volume]\nVolumeName={value}\n\n[Install]\nWantedBy=default.target\n");
	QuadletUnit {
		filename: format!("{}.volume", safe_unit_stem(name)),
		contents,
	}
}

pub(super) fn container_unit(
	name: &str,
	service: &Service,
	declared_volumes: &[&str],
	warnings: &mut Vec<String>,
) -> QuadletUnit {
	let mut unit = Section::new("Unit");
	unit.add("Description", format!("{name} (podup)"));
	for dep in service.depends_on.service_names() {
		unit.add("After", format!("{dep}.service"));
		if service.depends_on.required_for(&dep) {
			unit.add("Requires", format!("{dep}.service"));
		} else {
			unit.add("Wants", format!("{dep}.service"));
		}
	}

	let mut container = Section::new("Container");
	container.add("ContainerName", name.to_string());
	if let Some(image) = &service.image {
		container.add("Image", image.clone());
	}
	if let Some(hostname) = &service.hostname {
		container.add("HostName", hostname.clone());
	}
	if let Some(user) = &service.user {
		container.add("User", user.clone());
	}
	if let Some(wd) = &service.working_dir {
		container.add("WorkingDir", wd.clone());
	}
	if service.read_only == Some(true) {
		container.add("ReadOnly", "true".to_string());
	}
	if service.init == Some(true) {
		container.add("RunInit", "true".to_string());
	}

	match parse_ports(&service.ports) {
		Ok(ports) => {
			for p in ports {
				container.add("PublishPort", render_publish_port(&p));
			}
		}
		Err(_) => {
			// Fall back to the raw short forms so nothing is dropped.
			for port in &service.ports {
				if let PortMapping::Short(s) = port {
					container.add("PublishPort", s.clone());
				}
			}
		}
	}

	for (key, val) in sorted_pairs(service.environment.to_map()) {
		match val {
			Some(v) => container.add("Environment", format!("{key}={v}")),
			None => container.add("Environment", key),
		}
	}

	for vol in &service.volumes {
		container.add("Volume", render_volume(vol, declared_volumes));
	}
	for net in service.networks.names() {
		container.add("Network", format!("{net}.network"));
	}
	for (key, val) in sorted_label_pairs(service.labels.to_map()) {
		container.add("Label", format!("{key}={val}"));
	}
	for cap in &service.cap_add {
		container.add("AddCapability", cap.clone());
	}
	for cap in &service.cap_drop {
		container.add("DropCapability", cap.clone());
	}
	if let Some(entrypoint) = &service.entrypoint {
		container.add("Entrypoint", render_command(entrypoint));
	}
	if let Some(command) = &service.command {
		container.add("Exec", render_command(command));
	}

	let mut svc = Section::new("Service");
	if let Some(restart) = &service.restart {
		svc.add("Restart", render_restart(restart));
	}

	collect_warnings(name, service, warnings);

	let mut contents = String::new();
	contents.push_str(&unit.render());
	contents.push('\n');
	contents.push_str(&container.render());
	if !svc.is_empty() {
		contents.push('\n');
		contents.push_str(&svc.render());
	}
	contents.push_str("\n[Install]\nWantedBy=default.target\n");

	QuadletUnit {
		filename: format!("{}.container", safe_unit_stem(name)),
		contents,
	}
}
