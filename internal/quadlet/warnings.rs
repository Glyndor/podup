//! Report compose fields that are set but have no Quadlet mapping.

use crate::compose::types::{DependsOn, Service, ServiceCondition};

/// Warn for fields that are set but have no Quadlet mapping, so the operator
/// knows the generated unit is incomplete rather than discovering it at run
/// time.
pub(super) fn collect_warnings(name: &str, service: &Service, warnings: &mut Vec<String>) {
	let mut warn = |field: &str, detail: &str| {
		warnings.push(format!("{name}: {field} {detail}"));
	};
	if service.build.is_some() {
		warn(
			"build",
			"has no Quadlet equivalent; build the image first and set `image`",
		);
	}
	let replicas = service
		.scale
		.or(service.deploy.as_ref().and_then(|d| d.replicas));
	if replicas.is_some_and(|r| r > 1) {
		warn(
			"scale/replicas",
			"is ignored; Quadlet emits a single container per service",
		);
	}
	if !service.configs.is_empty() {
		warn("configs", "have no Quadlet equivalent and are skipped");
	}
	if !service.volumes_from.is_empty() {
		warn("volumes_from", "has no Quadlet equivalent and is skipped");
	}
	// `network_mode: host`/`none` map to `Network=`; other modes have no key.
	if service
		.network_mode
		.as_deref()
		.is_some_and(|m| m != "host" && m != "none")
	{
		warn(
			"network_mode",
			"is not mapped (only `host`/`none` are supported); use networks instead",
		);
	}
	if !service.profiles.is_empty() {
		warn("profiles", "have no Quadlet equivalent and are ignored");
	}
	if service.privileged == Some(true) {
		warn(
			"privileged",
			"is not mapped; add PodmanArgs manually if required",
		);
	}
	if !service.post_start.is_empty() {
		warn(
			"post_start",
			"hooks have no Quadlet equivalent and are skipped",
		);
	}
	if !service.pre_stop.is_empty() {
		warn(
			"pre_stop",
			"hooks have no Quadlet equivalent and are skipped",
		);
	}
	// systemd `After=`/`Requires=` order startup but cannot gate it on a
	// dependency becoming healthy or completing; those conditions are dropped.
	if let DependsOn::Map(deps) = &service.depends_on {
		if deps
			.values()
			.any(|c| c.condition != ServiceCondition::ServiceStarted)
		{
			warn(
				"depends_on",
				"condition service_healthy/service_completed_successfully is not enforceable in Quadlet; only start ordering is emitted",
			);
		}
	}

	// Fields that are honoured at runtime but have no [Container] Quadlet key and
	// no unambiguous PodmanArgs= fallback. Warn so the generated unit is never
	// silently incomplete; add the flag by hand if it is required.
	let skipped = "has no Quadlet equivalent and is skipped";
	if service.ipc.is_some() {
		warn("ipc", skipped);
	}
	if service.pid.is_some() {
		warn("pid", skipped);
	}
	if service.uts.is_some() {
		warn("uts", skipped);
	}
	if service.cgroup.is_some() {
		warn("cgroup", skipped);
	}
	if service.cgroup_parent.is_some() {
		warn("cgroup_parent", skipped);
	}
	if service.runtime.is_some() {
		warn("runtime", skipped);
	}
	if service.tty.is_some() {
		warn("tty", skipped);
	}
	if service.stdin_open.is_some() {
		warn("stdin_open", skipped);
	}
	if service.memswap_limit.is_some() {
		warn("memswap_limit", skipped);
	}
	if service.mem_reservation.is_some() {
		warn("mem_reservation", skipped);
	}
	if service.oom_kill_disable.is_some() {
		warn("oom_kill_disable", skipped);
	}
	if service.oom_score_adj.is_some() {
		warn("oom_score_adj", skipped);
	}
	if service.blkio_config.is_some() {
		warn("blkio_config", skipped);
	}
	if !service.label_file.to_list().is_empty() {
		warn("label_file", skipped);
	}
	// MAC addresses have no Quadlet key (service-level or per-network), and a
	// per-network value cannot be expressed via the whole-container PodmanArgs=.
	let has_network_mac = service
		.networks
		.names()
		.iter()
		.filter_map(|n| service.networks.config_for(n))
		.any(|c| c.mac_address.is_some());
	if service.mac_address.is_some() || has_network_mac {
		warn("mac_address", skipped);
	}
	// Only the first static IP across the service's networks is emitted; a second
	// one would need per-network IP scoping that Quadlet does not support.
	let static_ip_count = service
		.networks
		.names()
		.iter()
		.filter_map(|n| service.networks.config_for(n))
		.filter(|c| c.ipv4_address.is_some() || c.ipv6_address.is_some())
		.count();
	if static_ip_count > 1 {
		warn(
			"ipv4_address/ipv6_address",
			"is set on multiple networks; Quadlet emits only the first (no per-network IP scoping)",
		);
	}
}

#[cfg(test)]
mod tests {
	use crate::parse_str;
	use crate::quadlet::generate;

	#[test]
	fn warns_for_every_unmapped_field() {
		let yaml = r#"
services:
  everything:
    image: app:1.0
    build: .
    scale: 3
    privileged: true
    network_mode: "container:other"
    volumes_from:
      - other
    profiles:
      - debug
    healthcheck:
      test: ["CMD", "true"]
    secrets:
      - my_secret
    configs:
      - my_config
secrets:
  my_secret:
    file: ./s.txt
configs:
  my_config:
    file: ./c.txt
"#;
		let file = parse_str(yaml).unwrap();
		let warnings = generate(&file, "proj").warnings;
		let joined = warnings.join("\n");

		for field in [
			"build",
			"scale/replicas",
			"configs",
			"volumes_from",
			"network_mode",
			"profiles",
			"privileged",
		] {
			assert!(
				joined.contains(field),
				"missing warning for {field}; got:\n{joined}"
			);
		}
		// secrets are now mapped to Secret=, so they must NOT warn.
		assert!(
			!joined.contains("secrets"),
			"secrets should be mapped, not warned; got:\n{joined}"
		);
	}

	#[test]
	fn warns_for_silently_dropped_runtime_fields() {
		let yaml = r#"
services:
  s:
    image: x
    ipc: host
    pid: host
    uts: host
    cgroup: private
    cgroup_parent: /sys/fs/cgroup/p
    runtime: crun
    tty: true
    stdin_open: true
    mac_address: "02:42:ac:11:00:02"
    memswap_limit: 1g
    mem_reservation: 256m
    oom_kill_disable: true
    oom_score_adj: -500
    label_file:
      - ./labels.env
    blkio_config:
      weight: 300
"#;
		let file = parse_str(yaml).unwrap();
		let joined = generate(&file, "proj").warnings.join("\n");
		for field in [
			"ipc",
			"pid",
			"uts",
			"cgroup",
			"cgroup_parent",
			"runtime",
			"tty",
			"stdin_open",
			"mac_address",
			"memswap_limit",
			"mem_reservation",
			"oom_kill_disable",
			"oom_score_adj",
			"label_file",
			"blkio_config",
		] {
			assert!(
				joined.contains(field),
				"missing warning for {field}; got:\n{joined}"
			);
		}
	}

	#[test]
	fn warns_when_static_ip_set_on_multiple_networks() {
		let yaml = r#"
services:
  s:
    image: x
    networks:
      a:
        ipv4_address: 10.0.0.2
      b:
        ipv4_address: 10.0.1.2
networks:
  a:
  b:
"#;
		let file = parse_str(yaml).unwrap();
		let joined = generate(&file, "proj").warnings.join("\n");
		assert!(
			joined.contains("ipv4_address/ipv6_address"),
			"missing multi-network static-IP warning; got:\n{joined}"
		);
	}

	#[test]
	fn clean_service_warns_about_nothing() {
		let yaml = r#"
services:
  web:
    image: nginx:1.27
"#;
		let file = parse_str(yaml).unwrap();
		assert!(generate(&file, "proj").warnings.is_empty());
	}
}
