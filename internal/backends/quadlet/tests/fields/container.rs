use crate::parse_str;
use crate::quadlet::generate_at;
use crate::quadlet::tests::unit_named;

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
    x-podman-autoupdate: registry
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "proj-app.container").contents;
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
	// `x-podman-autoupdate: registry` reaches the Quadlet unit as
	// `AutoUpdate=registry`. Quadlet derives the `io.containers.autoupdate`
	// label from that key, so the generator must NOT emit a duplicate
	// `Label=io.containers.autoupdate=...` line (#1656).
	assert!(c.contains("AutoUpdate=registry"), "{c}");
	assert!(
		!c.contains("Label=io.containers.autoupdate="),
		"Quadlet derives the label from AutoUpdate=, so the Label= line \
		 must not be duplicated: {c}"
	);
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
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-app.container").contents;
	// Absolute, resolved against the compose base dir: systemd resolves a relative
	// `EnvironmentFile=` against the unit's own directory, not the project's, so
	// `./app.env` would never be found once installed. Built with `join` so the
	// separator matches the host (this test is not Unix-gated).
	// A backslash is escaped in the unit, so the host separator is doubled.
	let env_file = format!(
		"EnvironmentFile={}",
		std::path::Path::new("/srv/app")
			.join("app.env")
			.display()
			.to_string()
			.replace('\\', "\\\\")
	);
	for needle in [
		"ContainerName=custom",
		&env_file,
		"Tmpfs=/run",
		"Sysctl=net.core.somaxconn=1024",
		"Ulimit=nofile=1024:2048",
		"ShmSize=64m",
		"PidsLimit=100",
		"UserNS=keep-id",
		"PodmanArgs=--stop-signal=\"SIGTERM\"",
		"StopTimeout=30",
		"AddDevice=/dev/fuse",
		"DNS=1.1.1.1",
		"PodmanArgs=--add-host=\"db:10.0.0.2\"",
		"Annotation=run.oci.keep=1",
		"Network=host",
		"PodmanArgs=--memory=\"512m\"",
		"HealthCmd=curl -f http://localhost",
		"HealthInterval=5s",
		"HealthRetries=3",
		"StartLimitBurst=5",
	] {
		assert!(c.contains(needle), "missing `{needle}` in:\n{c}");
	}
}

#[test]
fn user_with_gid_splits_into_user_and_group() {
	let yaml = "services:\n  s:\n    image: x\n    user: \"1000:2000\"\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(c.contains("User=1000"));
	assert!(c.contains("Group=2000"));
	assert!(!c.contains("User=1000:2000"));
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
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	for needle in [
		"PodmanArgs=--group-add=\"audio\"",
		"ExposeHostPort=8080",
		"NoNewPrivileges=true",
		"SeccompProfile=/etc/seccomp.json",
		"SecurityLabelType=container_t",
		"Pull=always",
		"LogDriver=journald",
		"PodmanArgs=--log-opt=\"tag=mytag\"",
		"PodmanArgs=--network-alias=\"web-alias\"",
		"PodmanArgs=--memory=\"256m\"",
	] {
		assert!(c.contains(needle), "missing `{needle}` in:\n{c}");
	}
}

#[test]
fn security_opt_filetype_and_nested_map_to_keys() {
	let yaml = r#"
services:
  s:
    image: x
    security_opt:
      - "label=filetype:usr_t"
      - "label=nested"
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(c.contains("SecurityLabelFileType=usr_t"), "in:\n{c}");
	assert!(c.contains("SecurityLabelNested=true"), "in:\n{c}");
	assert!(
		!out.warnings.iter().any(|w| w.contains("security_opt")),
		"mapped labels must not warn; got: {:?}",
		out.warnings
	);
}
