//! Build the individual `.network`, `.volume` and `.container` units.

use crate::compose::types::{Command, PortMapping, RestartPolicy, Service, ServiceSecretRef};
use crate::ports::parse_ports;
use crate::size::parse_duration_secs;

use super::render::{
	render_command, render_publish_port, render_restart, render_volume, safe_unit_stem,
	sorted_label_pairs, sorted_pairs, Section,
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

/// Sanitize one `Secret=` option-list field: drop control characters and the
/// `,`/`=` separators so a hostile compose value cannot inject extra options.
fn secret_field(value: &str) -> String {
	value
		.chars()
		.filter(|c| !c.is_control() && *c != ',' && *c != '=')
		.collect()
}

/// Render a service `secrets:` entry into a Quadlet `Secret=` value
/// (`name[,target=,uid=,gid=,mode=]`).
fn render_secret(secret: &ServiceSecretRef) -> String {
	match secret {
		ServiceSecretRef::Short(name) => name.clone(),
		ServiceSecretRef::Long {
			source,
			target,
			uid,
			gid,
			mode,
		} => {
			// `Secret=` is a comma-separated `key=value` option list, so a `,`
			// or `=` embedded in any field would inject extra options. Strip
			// those (and control chars) from each value at the boundary.
			let mut s = secret_field(source);
			if let Some(t) = target {
				s.push_str(&format!(",target={}", secret_field(t)));
			}
			if let Some(u) = uid {
				s.push_str(&format!(",uid={}", secret_field(u)));
			}
			if let Some(g) = gid {
				s.push_str(&format!(",gid={}", secret_field(g)));
			}
			if let Some(m) = mode {
				s.push_str(&format!(",mode={m:o}"));
			}
			s
		}
	}
}

/// Map a single compose `security_opt` entry onto the dedicated Quadlet key
/// where one exists; unrecognized entries are reported rather than dropped.
fn map_security_opt(opt: &str, container: &mut Section, name: &str, warnings: &mut Vec<String>) {
	if let Some(rest) = opt.strip_prefix("no-new-privileges") {
		let val = rest.trim_start_matches([':', '=']);
		let enabled = val.is_empty() || val == "true";
		container.add("NoNewPrivileges", enabled.to_string());
	} else if let Some(profile) = opt.strip_prefix("seccomp=") {
		container.add("SeccompProfile", profile.to_string());
	} else if let Some(profile) = opt
		.strip_prefix("apparmor=")
		.or_else(|| opt.strip_prefix("apparmor:"))
	{
		container.add("AppArmor", profile.to_string());
	} else if let Some(label) = opt.strip_prefix("label=") {
		if label == "disable" {
			container.add("SecurityLabelDisable", "true".to_string());
		} else if let Some(t) = label.strip_prefix("type:") {
			container.add("SecurityLabelType", t.to_string());
		} else if let Some(l) = label.strip_prefix("level:") {
			container.add("SecurityLabelLevel", l.to_string());
		} else {
			warnings.push(format!(
				"{name}: security_opt 'label={label}' has no Quadlet key and is skipped"
			));
		}
	} else {
		warnings.push(format!(
			"{name}: security_opt '{opt}' has no Quadlet mapping and is skipped"
		));
	}
}

pub(super) fn network_unit(
	name: &str,
	project: &str,
	config: Option<&crate::compose::types::NetworkConfig>,
) -> QuadletUnit {
	let mut net = Section::new("Network");
	net.add("NetworkName", format!("{project}_{name}"));
	if let Some(cfg) = config {
		if let Some(driver) = &cfg.driver {
			net.add("Driver", driver.clone());
		}
		if cfg.internal == Some(true) {
			net.add("Internal", "true".to_string());
		}
		if cfg.enable_ipv6 == Some(true) {
			net.add("IPv6", "true".to_string());
		}
		if let Some(ipam) = &cfg.ipam {
			if let Some(ipam_driver) = &ipam.driver {
				net.add("IPAMDriver", ipam_driver.clone());
			}
		}
		for (key, val) in sorted_label_pairs(cfg.labels.to_map()) {
			net.add("Label", format!("{key}={val}"));
		}
	}
	let mut contents = net.render();
	contents.push_str("\n[Install]\nWantedBy=default.target\n");
	QuadletUnit {
		filename: format!("{}.network", safe_unit_stem(name)),
		contents,
	}
}

pub(super) fn volume_unit(
	name: &str,
	project: &str,
	config: Option<&crate::compose::types::VolumeConfig>,
) -> QuadletUnit {
	let mut vol = Section::new("Volume");
	vol.add("VolumeName", format!("{project}_{name}"));
	if let Some(cfg) = config {
		if let Some(driver) = &cfg.driver {
			vol.add("Driver", driver.clone());
		}
		for (key, val) in sorted_label_pairs(cfg.labels.to_map()) {
			vol.add("Label", format!("{key}={val}"));
		}
	}
	let mut contents = vol.render();
	contents.push_str("\n[Install]\nWantedBy=default.target\n");
	QuadletUnit {
		filename: format!("{}.volume", safe_unit_stem(name)),
		contents,
	}
}

pub(super) fn container_unit(
	name: &str,
	service: &Service,
	declared_volumes: &[&str],
	declared_networks: &[&str],
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
		// A declared (non-external) network is backed by a generated `.network`
		// unit; an external network is referenced by its existing name directly,
		// since no unit is emitted for it.
		if declared_networks.contains(&net.as_str()) {
			container.add("Network", format!("{net}.network"));
		} else {
			container.add("Network", net.clone());
		}
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
	for group in &service.group_add {
		container.add("GroupAdd", group.clone());
	}
	for port in &service.expose {
		container.add("ExposeHostPort", port.clone());
	}
	for net in service.networks.names() {
		if let Some(cfg) = service.networks.config_for(&net) {
			if let Some(aliases) = &cfg.aliases {
				for alias in aliases {
					container.add("NetworkAlias", alias.clone());
				}
			}
		}
	}
	for opt in &service.security_opt {
		map_security_opt(opt, &mut container, name, warnings);
	}
	if let Some(logging) = &service.logging {
		if let Some(driver) = &logging.driver {
			container.add("LogDriver", driver.clone());
		}
		for (key, val) in sorted_label_pairs(logging.options.clone()) {
			container.add("LogOpt", format!("{key}={val}"));
		}
	}
	if let Some(pull) = &service.pull_policy {
		container.add("Pull", pull.clone());
	}
	// `deploy.resources.limits.memory` is the modern equivalent of `mem_limit`.
	if service.mem_limit.is_none() {
		if let Some(mem) = service
			.deploy
			.as_ref()
			.and_then(|d| d.resources.as_ref())
			.and_then(|r| r.limits.as_ref())
			.and_then(|l| l.memory.as_ref())
		{
			container.add("Memory", mem.clone());
		}
	}
	for secret in &service.secrets {
		container.add("Secret", render_secret(secret));
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

#[cfg(test)]
mod tests {
	use super::{render_secret, secret_field};
	use crate::compose::types::ServiceSecretRef;

	#[test]
	fn secret_field_strips_separators_and_controls() {
		assert_eq!(secret_field("a,b=c\nd"), "abcd");
		assert_eq!(secret_field("plain"), "plain");
	}

	#[test]
	fn render_secret_cannot_inject_extra_options() {
		// A hostile target tries to smuggle a second option via `,` and `=`.
		let s = ServiceSecretRef::Long {
			source: "tok".into(),
			target: Some("/run/x,uid=0".into()),
			uid: None,
			gid: None,
			mode: None,
		};
		let out = render_secret(&s);
		// The injected `,uid=0` must be flattened into the target value, not a
		// separate option: exactly one comma (the legitimate `,target=`).
		assert_eq!(out.matches(',').count(), 1);
		assert_eq!(out, "tok,target=/run/xuid0");
	}
}
