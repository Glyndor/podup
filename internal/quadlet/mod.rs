//! Translate a parsed compose file into Podman Quadlet unit files.
//!
//! Quadlet is Podman's systemd integration: declarative `.container`,
//! `.network` and `.volume` units placed under
//! `~/.config/containers/systemd/` that a systemd generator turns into
//! services, so systemd owns the lifecycle (boot, restart, dependencies)
//! instead of a long-running `podup` process.
//!
//! This is an additive export path, not a replacement for the runner. It
//! maps the common compose fields and warns — loudly, never silently — for
//! every field that is set but has no Quadlet equivalent yet, so generated
//! units never quietly drop configuration.

mod render;
mod unit;
mod warnings;

use crate::compose::types::ComposeFile;
use unit::{container_unit, network_unit, volume_unit};

/// A single generated unit file: its name and full contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuadletUnit {
	/// File name, e.g. `web.container` or `db-data.volume`.
	pub filename: String,
	/// Full file contents, ending in a newline.
	pub contents: String,
}

/// The result of a generation run: the units plus any warnings about set but
/// unmapped fields.
#[derive(Debug, Clone, Default)]
pub struct QuadletOutput {
	/// Generated unit files, in a deterministic order.
	pub units: Vec<QuadletUnit>,
	/// Human-readable warnings for compose fields with no Quadlet mapping.
	pub warnings: Vec<String>,
}

/// Translate a compose file into Quadlet units for the given project name.
///
/// Emits one `.container` per service, one `.network` per declared network,
/// and one `.volume` per declared named volume. Replica scaling, build
/// services, and other fields without a Quadlet mapping are reported as
/// warnings rather than silently dropped.
pub fn generate(file: &ComposeFile, project: &str) -> QuadletOutput {
	let mut out = QuadletOutput::default();

	// External networks/volumes are assumed to pre-exist. Emitting a unit would
	// make systemd try to (re-)create them, so skip them here; the container unit
	// references such resources by their existing name instead.
	for (name, cfg) in &file.networks {
		if cfg.as_ref().is_some_and(|c| c.external == Some(true)) {
			continue;
		}
		out.units.push(network_unit(name, project, cfg.as_ref()));
	}
	for (name, cfg) in &file.volumes {
		if cfg.as_ref().is_some_and(|c| c.external == Some(true)) {
			continue;
		}
		out.units.push(volume_unit(name, project, cfg.as_ref()));
	}

	let declared_volumes: Vec<&str> = file
		.volumes
		.iter()
		.filter(|(_, cfg)| cfg.as_ref().is_none_or(|c| c.external != Some(true)))
		.map(|(name, _)| name.as_str())
		.collect();
	let declared_networks: Vec<&str> = file
		.networks
		.iter()
		.filter(|(_, cfg)| cfg.as_ref().is_none_or(|c| c.external != Some(true)))
		.map(|(name, _)| name.as_str())
		.collect();
	for (name, service) in &file.services {
		out.units.push(container_unit(
			name,
			service,
			&declared_volumes,
			&declared_networks,
			&mut out.warnings,
		));
	}

	out
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::parse_str;

	fn unit_named<'a>(out: &'a QuadletOutput, filename: &str) -> &'a QuadletUnit {
		out.units
			.iter()
			.find(|u| u.filename == filename)
			.unwrap_or_else(|| panic!("no unit named {filename}"))
	}

	#[test]
	fn generates_container_network_and_volume_units() {
		let yaml = r#"
services:
  web:
    image: nginx:1.27
    container_name: web
    ports:
      - "8080:80"
    environment:
      B_KEY: two
      A_KEY: one
    volumes:
      - data:/var/lib/data
    networks:
      - frontend
    restart: unless-stopped
    depends_on:
      - db
  db:
    image: postgres:16
volumes:
  data:
networks:
  frontend:
"#;
		let file = parse_str(yaml).unwrap();
		let out = generate(&file, "proj");

		let web = unit_named(&out, "web.container");
		assert!(web.contents.contains("Image=nginx:1.27"));
		assert!(web.contents.contains("ContainerName=web"));
		assert!(web.contents.contains("PublishPort=8080:80"));
		// Environment is emitted in sorted key order for determinism.
		let a = web.contents.find("Environment=A_KEY=one").unwrap();
		let b = web.contents.find("Environment=B_KEY=two").unwrap();
		assert!(a < b, "environment keys must be sorted");
		// Declared named volume is tied to its .volume unit.
		assert!(web.contents.contains("Volume=data.volume:/var/lib/data"));
		assert!(web.contents.contains("Network=frontend.network"));
		// unless-stopped maps to systemd Restart=always.
		assert!(web.contents.contains("Restart=always"));
		assert!(web.contents.contains("After=db.service"));
		assert!(web.contents.contains("WantedBy=default.target"));

		unit_named(&out, "db.container");
		assert!(unit_named(&out, "data.volume")
			.contents
			.contains("VolumeName=proj_data"));
		assert!(unit_named(&out, "frontend.network")
			.contents
			.contains("NetworkName=proj_frontend"));
	}

	#[test]
	fn warns_about_unmapped_build_field() {
		let yaml = r#"
services:
  app:
    build: .
    image: app:latest
"#;
		let file = parse_str(yaml).unwrap();
		let out = generate(&file, "proj");
		assert!(
			out.warnings.iter().any(|w| w.contains("build")),
			"a set build field must produce a warning"
		);
	}

	#[test]
	fn bind_path_volume_is_passed_through() {
		let yaml = r#"
services:
  web:
    image: nginx
    volumes:
      - ./html:/usr/share/nginx/html:ro
"#;
		let file = parse_str(yaml).unwrap();
		let out = generate(&file, "proj");
		let web = unit_named(&out, "web.container");
		assert!(web
			.contents
			.contains("Volume=./html:/usr/share/nginx/html:ro"));
	}

	#[test]
	fn maps_the_full_container_field_set() {
		let yaml = r#"
services:
  app:
    image: app:1.0
    hostname: app-host
    user: "1000:1000"
    working_dir: /srv
    read_only: true
    init: true
    entrypoint: ["/bin/sh", "-c"]
    command: server --port 9000
    labels:
      z_team: core
      a_tier: web
    cap_add:
      - NET_ADMIN
    cap_drop:
      - MKNOD
    ports:
      - target: 9000
        published: 9000
        protocol: udp
"#;
		let file = parse_str(yaml).unwrap();
		let out = generate(&file, "proj");
		let c = &unit_named(&out, "app.container").contents;
		assert!(c.contains("HostName=app-host"));
		// `user: "1000:1000"` splits into separate User=/Group= keys.
		assert!(c.contains("User=1000"));
		assert!(c.contains("Group=1000"));
		assert!(!c.contains("User=1000:1000"));
		assert!(c.contains("WorkingDir=/srv"));
		assert!(c.contains("ReadOnly=true"));
		assert!(c.contains("RunInit=true"));
		assert!(c.contains("Entrypoint=/bin/sh -c"));
		assert!(c.contains("Exec=server --port 9000"));
		assert!(c.contains("AddCapability=NET_ADMIN"));
		assert!(c.contains("DropCapability=MKNOD"));
		assert!(c.contains("PublishPort=9000:9000/udp"));
		// Labels sorted by key.
		let a = c.find("Label=a_tier=web").unwrap();
		let z = c.find("Label=z_team=core").unwrap();
		assert!(a < z, "labels must be sorted");
	}

	#[test]
	fn long_form_volume_with_named_source_and_readonly() {
		let yaml = r#"
services:
  db:
    image: postgres
    volumes:
      - type: volume
        source: pgdata
        target: /var/lib/postgresql/data
        read_only: true
volumes:
  pgdata:
"#;
		let file = parse_str(yaml).unwrap();
		let out = generate(&file, "proj");
		let c = &unit_named(&out, "db.container").contents;
		assert!(c.contains("Volume=pgdata.volume:/var/lib/postgresql/data:ro"));
	}

	#[test]
	fn restart_policies_map_to_systemd() {
		let cases = [
			("no", "Restart=no"),
			("always", "Restart=always"),
			("unless-stopped", "Restart=always"),
			("on-failure", "Restart=on-failure"),
		];
		for (policy, expected) in cases {
			let yaml = format!("services:\n  s:\n    image: x\n    restart: {policy}\n");
			let file = parse_str(&yaml).unwrap();
			let out = generate(&file, "p");
			assert!(
				unit_named(&out, "s.container").contents.contains(expected),
				"{policy} -> {expected}"
			);
		}
	}

	#[test]
	fn optional_dependency_uses_wants_not_requires() {
		let yaml = r#"
services:
  web:
    image: nginx
    depends_on:
      cache:
        condition: service_started
        required: false
  cache:
    image: redis
"#;
		let file = parse_str(yaml).unwrap();
		let out = generate(&file, "proj");
		let c = &unit_named(&out, "web.container").contents;
		assert!(c.contains("After=cache.service"));
		assert!(c.contains("Wants=cache.service"));
		assert!(!c.contains("Requires=cache.service"));
	}

	#[test]
	fn warns_for_every_unmapped_field() {
		let yaml = r#"
services:
  s:
    image: x
    network_mode: "container:other"
    privileged: true
    profiles: [debug]
    volumes_from:
      - other
    deploy:
      replicas: 3
"#;
		let file = parse_str(yaml).unwrap();
		let out = generate(&file, "p");
		let joined = out.warnings.join("\n");
		for needle in [
			"network_mode",
			"privileged",
			"profiles",
			"volumes_from",
			"scale/replicas",
		] {
			assert!(joined.contains(needle), "expected warning for {needle}");
		}
	}

	#[test]
	fn maps_extended_container_field_set() {
		let yaml = r#"
services:
  app:
    image: x
    container_name: custom
    env_file:
      - ./app.env
    tmpfs:
      - /run
    sysctls:
      net.core.somaxconn: "1024"
    ulimits:
      nofile:
        soft: 1024
        hard: 2048
    shm_size: 64m
    mem_limit: 512m
    pids_limit: 100
    userns_mode: keep-id
    stop_signal: SIGTERM
    stop_grace_period: 30s
    devices:
      - /dev/fuse
    dns:
      - 1.1.1.1
    extra_hosts:
      - "db:10.0.0.2"
    annotations:
      run.oci.keep: "1"
    network_mode: host
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost"]
      interval: 5s
      retries: 3
    restart: "on-failure:5"
"#;
		let file = parse_str(yaml).unwrap();
		let out = generate(&file, "p");
		let c = &unit_named(&out, "app.container").contents;
		for needle in [
			"ContainerName=custom",
			"EnvironmentFile=./app.env",
			"Tmpfs=/run",
			"Sysctl=net.core.somaxconn=1024",
			"Ulimit=nofile=1024:2048",
			"ShmSize=64m",
			"Memory=512m",
			"PidsLimit=100",
			"UserNS=keep-id",
			"StopSignal=SIGTERM",
			"StopTimeout=30",
			"AddDevice=/dev/fuse",
			"DNS=1.1.1.1",
			"AddHost=db:10.0.0.2",
			"Annotation=run.oci.keep=1",
			"Network=host",
			"HealthCmd=curl -f http://localhost",
			"HealthInterval=5s",
			"HealthRetries=3",
			"StartLimitBurst=5",
		] {
			assert!(c.contains(needle), "missing `{needle}` in:\n{c}");
		}
	}

	#[test]
	fn hostile_service_name_cannot_escape_output_directory() {
		// A compose key containing path separators must never yield a unit
		// file name that escapes the output directory.
		let yaml = "services:\n  ? \"../../evil\"\n  : { image: x }\n";
		let file = parse_str(yaml).unwrap();
		let out = generate(&file, "proj");
		let unit = &out.units[0];
		assert!(
			!unit.filename.contains('/') && !unit.filename.contains('\\'),
			"unit file name must be a single safe component, got {}",
			unit.filename
		);
		assert!(unit.filename.ends_with(".container"));
	}

	#[test]
	fn newline_in_value_cannot_inject_unit_directives() {
		// An environment value carrying a newline plus a forged directive must
		// be flattened to a single line, not injected as a new unit entry.
		let yaml =
			"services:\n  web:\n    image: x\n    environment:\n      EVIL: \"a\\nExecStartPre=/bin/rm -rf /\"\n";
		let file = parse_str(yaml).unwrap();
		let out = generate(&file, "proj");
		let c = &unit_named(&out, "web.container").contents;
		assert!(
			!c.lines().any(|l| l.starts_with("ExecStartPre")),
			"a newline in a value must not inject a directive line:\n{c}"
		);
	}

	#[test]
	fn ephemeral_published_port_omits_host_side() {
		let yaml = r#"
services:
  s:
    image: x
    ports:
      - "80"
"#;
		let file = parse_str(yaml).unwrap();
		let out = generate(&file, "p");
		let c = &unit_named(&out, "s.container").contents;
		assert!(c.contains("PublishPort=80"));
		assert!(!c.contains("PublishPort=:80"));
	}

	#[test]
	fn external_network_and_volume_emit_no_unit_and_use_bare_name() {
		let yaml = r#"
services:
  web:
    image: nginx
    networks:
      - extnet
    volumes:
      - extvol:/data
networks:
  extnet:
    external: true
volumes:
  extvol:
    external: true
"#;
		let file = parse_str(yaml).unwrap();
		let out = generate(&file, "proj");
		// No unit is generated for external resources.
		assert!(!out.units.iter().any(|u| u.filename == "extnet.network"));
		assert!(!out.units.iter().any(|u| u.filename == "extvol.volume"));
		// The container references them by their existing name, not `.network`/`.volume`.
		let c = &unit_named(&out, "web.container").contents;
		assert!(c.contains("Network=extnet"));
		assert!(!c.contains("Network=extnet.network"));
		assert!(c.contains("Volume=extvol:/data"));
		assert!(!c.contains("extvol.volume"));
	}

	#[test]
	fn user_with_gid_splits_into_user_and_group() {
		let yaml = "services:\n  s:\n    image: x\n    user: \"1000:2000\"\n";
		let file = parse_str(yaml).unwrap();
		let out = generate(&file, "p");
		let c = &unit_named(&out, "s.container").contents;
		assert!(c.contains("User=1000"));
		assert!(c.contains("Group=2000"));
		assert!(!c.contains("User=1000:2000"));
	}

	#[test]
	fn long_form_bind_selinux_and_propagation_preserved() {
		let yaml = r#"
services:
  s:
    image: x
    volumes:
      - type: bind
        source: /host/data
        target: /data
        bind:
          selinux: z
          propagation: rshared
"#;
		let file = parse_str(yaml).unwrap();
		let out = generate(&file, "p");
		let c = &unit_named(&out, "s.container").contents;
		assert!(
			c.contains("Volume=/host/data:/data:z,rshared"),
			"selinux/propagation must be preserved; got:\n{c}"
		);
	}

	#[test]
	fn service_secret_maps_to_secret_key() {
		let yaml = r#"
services:
  s:
    image: x
    secrets:
      - tok
      - source: cred
        target: /run/cred
        uid: "100"
secrets:
  tok:
    file: ./tok
  cred:
    file: ./cred
"#;
		let file = parse_str(yaml).unwrap();
		let out = generate(&file, "p");
		let c = &unit_named(&out, "s.container").contents;
		assert!(c.contains("Secret=tok"));
		assert!(c.contains("Secret=cred,target=/run/cred,uid=100"));
		assert!(!out.warnings.iter().any(|w| w.contains("secrets")));
	}

	#[test]
	fn maps_previously_dropped_container_fields() {
		let yaml = r#"
services:
  s:
    image: x
    group_add:
      - audio
    expose:
      - "8080"
    security_opt:
      - "no-new-privileges:true"
      - "seccomp=/etc/seccomp.json"
      - "label=type:container_t"
    pull_policy: always
    logging:
      driver: journald
      options:
        tag: mytag
    networks:
      net:
        aliases:
          - web-alias
    deploy:
      resources:
        limits:
          memory: 256m
networks:
  net:
"#;
		let file = parse_str(yaml).unwrap();
		let out = generate(&file, "p");
		let c = &unit_named(&out, "s.container").contents;
		for needle in [
			"GroupAdd=audio",
			"ExposeHostPort=8080",
			"NoNewPrivileges=true",
			"SeccompProfile=/etc/seccomp.json",
			"SecurityLabelType=container_t",
			"Pull=always",
			"LogDriver=journald",
			"LogOpt=tag=mytag",
			"NetworkAlias=web-alias",
			"Memory=256m",
		] {
			assert!(c.contains(needle), "missing `{needle}` in:\n{c}");
		}
	}

	#[test]
	fn network_and_volume_units_carry_config_keys() {
		let yaml = r#"
services:
  s:
    image: x
networks:
  net:
    driver: bridge
    internal: true
    enable_ipv6: true
    labels:
      tier: net
volumes:
  vol:
    driver: local
    labels:
      tier: vol
"#;
		let file = parse_str(yaml).unwrap();
		let out = generate(&file, "p");
		let net = &unit_named(&out, "net.network").contents;
		assert!(net.contains("Driver=bridge"));
		assert!(net.contains("Internal=true"));
		assert!(net.contains("IPv6=true"));
		assert!(net.contains("Label=tier=net"));
		let vol = &unit_named(&out, "vol.volume").contents;
		assert!(vol.contains("Driver=local"));
		assert!(vol.contains("Label=tier=vol"));
	}
}
