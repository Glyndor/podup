//! Build the `.container` unit for a service.

use crate::compose::types::{PortMapping, RestartPolicy, Service};
use crate::ports::parse_ports;
use crate::size::parse_duration_secs;

use super::health::render_healthcheck;
use super::security::{map_security_opt, render_secret};
use super::{
	collect_warnings, render_command, render_publish_port, render_restart, render_tmpfs_mount,
	render_volume, safe_unit_stem, sorted_label_pairs, sorted_pairs, QuadletUnit, Section,
};

pub(crate) fn container_unit(
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
		// A long-form `type: tmpfs` mount maps to `Tmpfs=`, not `Volume=`
		// (which would persist it as a volume rather than an in-memory fs).
		if let Some(t) = render_tmpfs_mount(vol) {
			container.add("Tmpfs", t);
		} else {
			container.add("Volume", render_volume(vol, declared_volumes));
		}
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
		// `Memory=` is not a valid Quadlet key in Podman 5.x (the generator
		// rejects the unit); express the limit as a raw podman flag.
		container.add("PodmanArgs", format!("--memory={mem}"));
	}
	// CPU limits have no native [Container] Quadlet key (unlike Memory=/
	// PidsLimit=), so they go through PodmanArgs=, mirroring --memory above.
	// `cpus` falls back to the modern `deploy.resources.limits.cpus`.
	let deploy_cpus = service
		.deploy
		.as_ref()
		.and_then(|d| d.resources.as_ref())
		.and_then(|r| r.limits.as_ref())
		.and_then(|l| l.cpus.as_deref());
	if let Some(c) = service.cpus.as_deref().or(deploy_cpus) {
		container.add("PodmanArgs", format!("--cpus={c}"));
	}
	if let Some(cs) = &service.cpuset {
		container.add("PodmanArgs", format!("--cpuset-cpus={cs}"));
	}
	if let Some(sh) = service.cpu_shares {
		container.add("PodmanArgs", format!("--cpu-shares={sh}"));
	}
	if let Some(q) = service.cpu_quota {
		container.add("PodmanArgs", format!("--cpu-quota={q}"));
	}
	if let Some(p) = service.cpu_period {
		container.add("PodmanArgs", format!("--cpu-period={p}"));
	}
	// `deploy.resources.limits.pids` is the modern equivalent of `pids_limit`.
	let deploy_pids = service
		.deploy
		.as_ref()
		.and_then(|d| d.resources.as_ref())
		.and_then(|r| r.limits.as_ref())
		.and_then(|l| l.pids);
	if let Some(pids) = service.pids_limit {
		container.add("PidsLimit", pids.to_string());
	} else if let Some(pids) = deploy_pids {
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
	// `network_mode: host`/`none` map to `Network=host`/`Network=none`; other
	// modes (service:/container:) have no Quadlet key and are reported by
	// collect_warnings.
	match service.network_mode.as_deref() {
		Some("host") => container.add("Network", "host".to_string()),
		Some("none") => container.add("Network", "none".to_string()),
		_ => {}
	}
	for group in &service.group_add {
		container.add("GroupAdd", group.clone());
	}
	for port in &service.expose {
		container.add("ExposeHostPort", port.clone());
	}
	// `IP=`/`IP6=` are single-valued per container, so the first static address
	// declared across the service's networks wins (Quadlet has no per-network IP
	// scoping); a second one is reported by collect_warnings.
	let mut static_ip: Option<&str> = None;
	let mut static_ip6: Option<&str> = None;
	for net in service.networks.names() {
		if let Some(cfg) = service.networks.config_for(&net) {
			if let Some(aliases) = &cfg.aliases {
				for alias in aliases {
					container.add("NetworkAlias", alias.clone());
				}
			}
			if static_ip.is_none() {
				static_ip = cfg.ipv4_address.as_deref();
			}
			if static_ip6.is_none() {
				static_ip6 = cfg.ipv6_address.as_deref();
			}
		}
	}
	if let Some(ip) = static_ip {
		container.add("IP", ip.to_string());
	}
	if let Some(ip6) = static_ip6 {
		container.add("IP6", ip6.to_string());
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
			container.add("PodmanArgs", format!("--memory={mem}"));
		}
	}
	for secret in &service.secrets {
		container.add("Secret", render_secret(secret));
	}
	render_healthcheck(name, service, &mut container, warnings);

	let mut svc = Section::new("Service");
	if let Some(restart) = &service.restart {
		svc.add("Restart", render_restart(restart));
		if let RestartPolicy::OnFailure {
			max_attempts: Some(n),
		} = restart
		{
			svc.add("StartLimitBurst", n.to_string());
		}
	} else if let Some(rp) = service
		.deploy
		.as_ref()
		.and_then(|d| d.restart_policy.as_ref())
	{
		// `deploy.restart_policy` is the modern equivalent of the service-level
		// `restart:` string; its `condition` maps onto the systemd `Restart=`
		// values and `max_attempts`/`window` onto the start-limit window.
		let restart = match rp.condition.as_deref() {
			Some("none") => "no",
			Some("on-failure") => "on-failure",
			// "any" (the compose default) and any unknown value restart always.
			_ => "always",
		};
		svc.add("Restart", restart.to_string());
		if let Some(n) = rp.max_attempts {
			svc.add("StartLimitBurst", n.to_string());
			if let Some(secs) = rp.window.as_deref().and_then(parse_duration_secs) {
				svc.add("StartLimitIntervalSec", secs.to_string());
			}
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
