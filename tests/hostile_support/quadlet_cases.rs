//! The compose fields the hostile Quadlet test attacks, one case per field.
//!
//! Declared by `hostile_quadlet_units.rs` with
//! `#[path = "hostile_support/quadlet_cases.rs"] mod quadlet_cases;`.

/// One compose field to attack. `yaml` receives the YAML-quoted payload.
pub struct Case {
	pub name: &'static str,
	pub yaml: fn(&str) -> String,
}

fn svc(extra: &str) -> String {
	format!("services:\n  web:\n    image: nginx\n{extra}")
}

pub fn cases() -> Vec<Case> {
	vec![
		Case {
			name: "image",
			yaml: |p| format!("services:\n  web:\n    image: {p}\n"),
		},
		Case {
			name: "command string",
			yaml: |p| svc(&format!("    command: {p}\n")),
		},
		Case {
			name: "command list",
			yaml: |p| svc(&format!("    command: [{p}, {p}]\n")),
		},
		Case {
			name: "entrypoint string",
			yaml: |p| svc(&format!("    entrypoint: {p}\n")),
		},
		Case {
			name: "entrypoint list",
			yaml: |p| svc(&format!("    entrypoint: [{p}]\n")),
		},
		Case {
			name: "environment value",
			yaml: |p| svc(&format!("    environment:\n      K: {p}\n")),
		},
		Case {
			name: "environment key",
			yaml: |p| svc(&format!("    environment:\n      {p}: v\n")),
		},
		Case {
			name: "environment list",
			yaml: |p| svc(&format!("    environment: [{p}]\n")),
		},
		Case {
			name: "label value",
			yaml: |p| svc(&format!("    labels:\n      k: {p}\n")),
		},
		Case {
			name: "label key",
			yaml: |p| svc(&format!("    labels:\n      {p}: v\n")),
		},
		Case {
			name: "volume short",
			yaml: |p| svc(&format!("    volumes: [{p}]\n")),
		},
		Case {
			name: "volume long",
			yaml: |p| {
				svc(&format!(
					"    volumes:\n      - type: bind\n        source: {p}\n        target: {p}\n"
				))
			},
		},
		Case {
			name: "ports",
			yaml: |p| svc(&format!("    ports: [{p}]\n")),
		},
		Case {
			name: "container_name",
			yaml: |p| svc(&format!("    container_name: {p}\n")),
		},
		Case {
			name: "hostname",
			yaml: |p| svc(&format!("    hostname: {p}\n")),
		},
		Case {
			name: "working_dir",
			yaml: |p| svc(&format!("    working_dir: {p}\n")),
		},
		Case {
			name: "user",
			yaml: |p| svc(&format!("    user: {p}\n")),
		},
		Case {
			name: "healthcheck test",
			yaml: |p| svc(&format!("    healthcheck:\n      test: [CMD, {p}]\n")),
		},
		Case {
			name: "healthcheck shell",
			yaml: |p| svc(&format!("    healthcheck:\n      test: [CMD-SHELL, {p}]\n")),
		},
		Case {
			name: "healthcheck interval",
			yaml: |p| {
				svc(&format!(
					"    healthcheck:\n      test: [CMD, a]\n      interval: {p}\n"
				))
			},
		},
		Case {
			name: "build context",
			yaml: |p| format!("services:\n  web:\n    build:\n      context: {p}\n"),
		},
		Case {
			name: "build dockerfile",
			yaml: |p| {
				format!("services:\n  web:\n    build:\n      context: .\n      dockerfile: {p}\n")
			},
		},
		Case {
			name: "build arg",
			yaml: |p| {
				format!("services:\n  web:\n    build:\n      context: .\n      args:\n        A: {p}\n")
			},
		},
		Case {
			name: "build arg key",
			yaml: |p| {
				format!("services:\n  web:\n    build:\n      context: .\n      args:\n        {p}: v\n")
			},
		},
		Case {
			name: "build target",
			yaml: |p| {
				format!("services:\n  web:\n    build:\n      context: .\n      target: {p}\n")
			},
		},
		Case {
			name: "secret short",
			yaml: |p| {
				format!("{}secrets: [{p}]\n", svc("    ")).replacen(
					"    secrets:",
					"    secrets:",
					1,
				)
			},
		},
		Case {
			name: "secret file",
			yaml: |p| {
				format!("services:\n  web:\n    image: nginx\n    secrets: [s]\nsecrets:\n  s:\n    file: {p}\n")
			},
		},
		Case {
			name: "secret name",
			yaml: |p| {
				format!("services:\n  web:\n    image: nginx\n    secrets: [s]\nsecrets:\n  s:\n    file: /etc/hostname\n    name: {p}\n")
			},
		},
		Case {
			name: "network name",
			yaml: |p| {
				format!("services:\n  web:\n    image: nginx\n    networks: [n]\nnetworks:\n  n:\n    name: {p}\n")
			},
		},
		Case {
			name: "network key",
			yaml: |p| {
				format!("services:\n  web:\n    image: nginx\n    networks: [{p}]\nnetworks:\n  {p}: {{}}\n")
			},
		},
		Case {
			name: "network driver opt",
			yaml: |p| {
				format!("services:\n  web:\n    image: nginx\n    networks: [n]\nnetworks:\n  n:\n    driver_opts:\n      k: {p}\n")
			},
		},
		Case {
			name: "network ipam",
			yaml: |p| {
				format!("services:\n  web:\n    image: nginx\n    networks: [n]\nnetworks:\n  n:\n    ipam:\n      config:\n        - subnet: {p}\n          gateway: {p}\n")
			},
		},
		Case {
			name: "volume key",
			yaml: |p| {
				format!("services:\n  web:\n    image: nginx\n    volumes: [\"v:/d\"]\nvolumes:\n  v:\n    name: {p}\n")
			},
		},
		Case {
			name: "volume driver opt",
			yaml: |p| {
				format!("services:\n  web:\n    image: nginx\n    volumes: [\"v:/d\"]\nvolumes:\n  v:\n    driver_opts:\n      k: {p}\n")
			},
		},
		Case {
			name: "volume label",
			yaml: |p| {
				format!("services:\n  web:\n    image: nginx\n    volumes: [\"v:/d\"]\nvolumes:\n  v:\n    labels:\n      k: {p}\n")
			},
		},
		Case {
			name: "service name",
			yaml: |p| format!("services:\n  {p}:\n    image: nginx\n"),
		},
		Case {
			name: "depends_on name",
			yaml: |p| {
				format!("services:\n  web:\n    image: nginx\n    depends_on: [{p}]\n  {p}:\n    image: x\n")
			},
		},
		Case {
			name: "cap_add",
			yaml: |p| svc(&format!("    cap_add: [{p}]\n")),
		},
		Case {
			name: "devices",
			yaml: |p| svc(&format!("    devices: [{p}]\n")),
		},
		Case {
			name: "dns",
			yaml: |p| svc(&format!("    dns: [{p}]\n")),
		},
		Case {
			name: "extra_hosts",
			yaml: |p| svc(&format!("    extra_hosts: [{p}]\n")),
		},
		Case {
			name: "sysctls",
			yaml: |p| svc(&format!("    sysctls:\n      k: {p}\n")),
		},
		Case {
			name: "tmpfs",
			yaml: |p| svc(&format!("    tmpfs: [{p}]\n")),
		},
		Case {
			name: "security_opt",
			yaml: |p| svc(&format!("    security_opt: [{p}]\n")),
		},
		Case {
			name: "ulimits",
			yaml: |p| svc(&format!("    ulimits:\n      {p}: 1\n")),
		},
		Case {
			name: "stop_signal",
			yaml: |p| svc(&format!("    stop_signal: {p}\n")),
		},
		Case {
			name: "pull_policy",
			yaml: |p| svc(&format!("    pull_policy: {p}\n")),
		},
		Case {
			name: "network_mode",
			yaml: |p| svc(&format!("    network_mode: {p}\n")),
		},
		Case {
			name: "pid",
			yaml: |p| svc(&format!("    pid: {p}\n")),
		},
		Case {
			name: "shm_size",
			yaml: |p| svc(&format!("    shm_size: {p}\n")),
		},
		Case {
			name: "mem_limit",
			yaml: |p| svc(&format!("    mem_limit: {p}\n")),
		},
		Case {
			name: "platform",
			yaml: |p| svc(&format!("    platform: {p}\n")),
		},
		Case {
			name: "pod",
			yaml: |p| svc(&format!("    x-podman:\n      pod: {p}\n")),
		},
		Case {
			name: "env_file",
			yaml: |p| svc(&format!("    env_file: [{p}]\n")),
		},
		Case {
			name: "annotations",
			yaml: |p| svc(&format!("    annotations:\n      k: {p}\n")),
		},
	]
}
