//! Build the individual `.network`, `.volume` and `.container` units.

use crate::compose::types::{Command, PortMapping, RestartPolicy, Service};
use crate::ports::parse_ports;
use crate::size::parse_duration_secs;

use super::render::{
	render_command, render_publish_port, render_restart, render_volume, safe_unit_stem,
	sanitize_value, sorted_label_pairs, sorted_pairs, Section,
};
use super::warnings::collect_warnings;
use super::QuadletUnit;

/// Map a compose `healthcheck:` onto the Quadlet `Health*=` keys. A disabled
/// healthcheck emits `HealthCmd=none`; otherwise the compose test (with any
/// leading `CMD`/`CMD-SHELL`/`NONE` sentinel stripped) and the timing fields
/// are rendered.
fn render_healthcheck(service: &Service, container: &mut Section) {
	let Some(hc) = &service.healthcheck else {
		return;
	};
	if hc.is_disabled() {
		container.add("HealthCmd", "none".to_string());
		return;
	}
	if let Some(test) = &hc.test {
		let cmd = match test {
			Command::Shell(s) => s.clone(),
			Command::Exec(parts) => {
				let body = match parts.first().map(String::as_str) {
					Some("CMD") | Some("CMD-SHELL") | Some("NONE") => &parts[1..],
					_ => &parts[..],
				};
				body.join(" ")
			}
		};
		if !cmd.is_empty() {
			container.add("HealthCmd", cmd);
		}
	}
	if let Some(v) = &hc.interval {
		container.add("HealthInterval", v.clone());
	}
	if let Some(v) = &hc.timeout {
		container.add("HealthTimeout", v.clone());
	}
	if let Some(v) = hc.retries {
		container.add("HealthRetries", v.to_string());
	}
	if let Some(v) = &hc.start_period {
		container.add("HealthStartPeriod", v.clone());
	}
	if let Some(v) = &hc.start_interval {
		container.add("HealthStartupInterval", v.clone());
	}
}

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
	container.add(
		"ContainerName",
		service
			.container_name
			.clone()
			.unwrap_or_else(|| name.to_string()),
	);
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

	for ann in sorted_label_pairs(service.annotations.to_map()) {
		container.add("Annotation", format!("{}={}", ann.0, ann.1));
	}
	for entry in service.env_file.to_entries() {
		container.add("EnvironmentFile", entry.path().to_string());
	}
	for t in service.tmpfs.to_list() {
		container.add("Tmpfs", t);
	}
	for (key, val) in sorted_label_pairs(service.sysctls.to_map()) {
		container.add("Sysctl", format!("{key}={val}"));
	}
	for (name, limit) in &service.ulimits {
		let soft = limit.soft();
		let hard = limit.hard();
		let value = if soft == hard {
			format!("{name}={soft}")
		} else {
			format!("{name}={soft}:{hard}")
		};
		container.add("Ulimit", value);
	}
	for dev in &service.devices {
		container.add("AddDevice", dev.clone());
	}
	for host in &service.extra_hosts {
		container.add("AddHost", host.clone());
	}
	for d in service.dns.to_list() {
		container.add("DNS", d);
	}
	for d in service.dns_search.to_list() {
		container.add("DNSSearch", d);
	}
	for d in service.dns_opt.to_list() {
		container.add("DNSOption", d);
	}
	if let Some(shm) = &service.shm_size {
		container.add("ShmSize", shm.clone());
	}
	if let Some(mem) = &service.mem_limit {
		container.add("Memory", mem.clone());
	}
	if let Some(pids) = service.pids_limit {
		container.add("PidsLimit", pids.to_string());
	}
	if let Some(userns) = &service.userns_mode {
		container.add("UserNS", userns.clone());
	}
	if let Some(signal) = &service.stop_signal {
		container.add("StopSignal", signal.clone());
	}
	if let Some(grace) = &service.stop_grace_period {
		if let Some(secs) = parse_duration_secs(grace) {
			container.add("StopTimeout", secs.to_string());
		}
	}
	// `network_mode: host` maps to `Network=host`; other modes (service:/
	// container:) have no Quadlet key and are reported by collect_warnings.
	if service.network_mode.as_deref() == Some("host") {
		container.add("Network", "host".to_string());
	}
	render_healthcheck(service, &mut container);

	let mut svc = Section::new("Service");
	if let Some(restart) = &service.restart {
		svc.add("Restart", render_restart(restart));
		if let RestartPolicy::OnFailure {
			max_attempts: Some(n),
		} = restart
		{
			svc.add("StartLimitBurst", n.to_string());
		}
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
