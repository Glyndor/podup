use super::unit_named;
use crate::parse_str;
use crate::quadlet::{generate, QuadletOutput, QuadletUnit};

#[test]
fn container_name_defaults_to_project_prefixed() {
	// A service with no explicit `container_name:` must default to
	// `{project}-{service}`, matching how `up` names the running container,
	// rather than a bare `web` that would collide across projects.
	let file = parse_str("services:\n  web:\n    image: nginx\n").unwrap();
	let out = generate(&file, "proj");
	let web = unit_named(&out, "proj-web.container");
	assert!(web.contents.contains("ContainerName=proj-web"));
}

#[test]
fn duplicate_filename_detects_collision() {
	let mk = |n: &str| QuadletUnit {
		filename: n.to_string(),
		contents: String::new(),
	};
	let mut out = QuadletOutput {
		units: vec![mk("web.container"), mk("db.volume")],
		..Default::default()
	};
	assert_eq!(out.duplicate_filename(), None);
	out.units.push(mk("web.container"));
	assert_eq!(out.duplicate_filename(), Some("web.container"));
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

	let web = unit_named(&out, "proj-web.container");
	assert!(web.contents.contains("Image=nginx:1.27"));
	assert!(web.contents.contains("ContainerName=web"));
	assert!(web.contents.contains("PublishPort=8080:80"));
	// Environment is emitted in sorted key order for determinism.
	let a = web.contents.find("Environment=A_KEY=one").unwrap();
	let b = web.contents.find("Environment=B_KEY=two").unwrap();
	assert!(a < b, "environment keys must be sorted");
	// Declared named volume is tied to its .volume unit.
	assert!(web
		.contents
		.contains("Volume=proj-data.volume:/var/lib/data"));
	assert!(web.contents.contains("Network=proj-frontend.network"));
	// unless-stopped maps to systemd Restart=always.
	assert!(web.contents.contains("Restart=always"));
	assert!(web.contents.contains("After=proj-db.service"));
	assert!(web.contents.contains("WantedBy=default.target"));

	unit_named(&out, "proj-db.container");
	assert!(unit_named(&out, "proj-data.volume")
		.contents
		.contains("VolumeName=proj_data"));
	assert!(unit_named(&out, "proj-frontend.network")
		.contents
		.contains("NetworkName=proj_frontend"));
}

#[test]
fn build_field_emits_a_build_unit() {
	let yaml = r#"
services:
  app:
    build:
      context: ./src
      dockerfile: Dockerfile.app
      target: runtime
    image: app:latest
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate(&file, "proj");
	// A `.build` unit is generated (no longer just a warning) and the container
	// references it so Quadlet builds before running.
	let build = out
		.units
		.iter()
		.find(|u| u.filename == "proj-app.build")
		.expect("a build service must emit an app.build unit");
	assert!(build.contents.contains("[Build]"));
	assert!(build.contents.contains("ImageTag=app:latest"));
	assert!(build.contents.contains("SetWorkingDirectory=./src"));
	assert!(build.contents.contains("File=Dockerfile.app"));
	assert!(build.contents.contains("Target=runtime"));
	let container = out
		.units
		.iter()
		.find(|u| u.filename == "proj-app.container")
		.unwrap();
	assert!(container.contents.contains("Image=proj-app.build"));
	assert!(!out.warnings.iter().any(|w| w.contains("build")));
}

#[test]
fn inline_dockerfile_build_warns_and_emits_no_build_unit() {
	let yaml = "services:\n  app:\n    build:\n      dockerfile_inline: \"FROM alpine\"\n";
	let file = parse_str(yaml).unwrap();
	let out = generate(&file, "proj");
	assert!(!out.units.iter().any(|u| u.filename == "proj-app.build"));
	assert!(out.warnings.iter().any(|w| w.contains("dockerfile_inline")));
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
	let web = unit_named(&out, "proj-web.container");
	assert!(web
		.contents
		.contains("Volume=./html:/usr/share/nginx/html:ro"));
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
	let c = &unit_named(&out, "proj-db.container").contents;
	assert!(c.contains("Volume=proj-pgdata.volume:/var/lib/postgresql/data:ro"));
}

#[test]
fn warns_for_every_unmapped_field() {
	let yaml = r#"
services:
  s:
    image: x
    network_mode: "bridge:custom"
    profiles: [debug]
    volumes_from:
      - other
    deploy:
      replicas: 3
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate(&file, "p");
	let joined = out.warnings.join("\n");
	for needle in ["network_mode", "profiles", "volumes_from", "scale/replicas"] {
		assert!(joined.contains(needle), "expected warning for {needle}");
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
	let c = &unit_named(&out, "proj-web.container").contents;
	assert!(
		!c.lines().any(|l| l.starts_with("ExecStartPre")),
		"a newline in a value must not inject a directive line:\n{c}"
	);
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
	assert!(!out
		.units
		.iter()
		.any(|u| u.filename == "proj-extnet.network"));
	assert!(!out.units.iter().any(|u| u.filename == "proj-extvol.volume"));
	// The container references them by their existing name, not `.network`/`.volume`.
	let c = &unit_named(&out, "proj-web.container").contents;
	assert!(c.contains("Network=extnet"));
	assert!(!c.contains("Network=proj-extnet.network"));
	assert!(c.contains("Volume=extvol:/data"));
	assert!(!c.contains("extvol.volume"));
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
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(
		c.contains("Volume=/host/data:/data:z,rshared"),
		"selinux/propagation must be preserved; got:\n{c}"
	);
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
	let net = &unit_named(&out, "p-net.network").contents;
	assert!(net.contains("Driver=bridge"));
	assert!(net.contains("Internal=true"));
	assert!(net.contains("IPv6=true"));
	assert!(net.contains("Label=tier=net"));
	let vol = &unit_named(&out, "p-vol.volume").contents;
	assert!(vol.contains("Driver=local"));
	assert!(vol.contains("Label=tier=vol"));
}

#[test]
fn network_unit_emits_ipam_pools_options_and_custom_name() {
	let yaml = r#"
services:
  s:
    image: x
networks:
  net:
    name: custom-net
    driver_opts:
      mtu: "9000"
      com.docker.network.bridge.name: br0
    ipam:
      driver: host-local
      config:
        - subnet: 10.7.0.0/16
          gateway: 10.7.0.1
          ip_range: 10.7.0.128/25
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate(&file, "p");
	let net = &unit_named(&out, "p-net.network").contents;
	// A custom name overrides the project-prefixed default.
	assert!(net.contains("NetworkName=custom-net"), "in:\n{net}");
	assert!(!net.contains("NetworkName=p_net"), "in:\n{net}");
	assert!(net.contains("IPAMDriver=host-local"), "in:\n{net}");
	assert!(net.contains("Subnet=10.7.0.0/16"), "in:\n{net}");
	assert!(net.contains("Gateway=10.7.0.1"), "in:\n{net}");
	assert!(net.contains("IPRange=10.7.0.128/25"), "in:\n{net}");
	// Each driver option is its own Options= line (Quadlet maps one Options= to
	// one `--opt`); a comma-joined value would be a single malformed option.
	assert!(net.contains("Options=mtu=9000"), "in:\n{net}");
	assert!(
		net.contains("Options=com.docker.network.bridge.name=br0"),
		"in:\n{net}"
	);
}

#[test]
fn volume_unit_emits_options_and_custom_name() {
	let yaml = r#"
services:
  s:
    image: x
volumes:
  vol:
    name: custom-vol
    driver: local
    driver_opts:
      type: nfs
      device: ":/exports"
      o: "addr=10.0.0.1,rw"
      custom: extra
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate(&file, "p");
	let vol = &unit_named(&out, "p-vol.volume").contents;
	assert!(vol.contains("VolumeName=custom-vol"), "in:\n{vol}");
	assert!(!vol.contains("VolumeName=p_vol"), "in:\n{vol}");
	// `local` driver opts map to dedicated keys; `o` is a single mount-option
	// string. Options= without a Device= would be rejected by Quadlet.
	assert!(vol.contains("Type=nfs"), "in:\n{vol}");
	assert!(vol.contains("Device=:/exports"), "in:\n{vol}");
	assert!(vol.contains("Options=addr=10.0.0.1,rw"), "in:\n{vol}");
	// An option with no dedicated key falls back to PodmanArgs=--opt.
	assert!(vol.contains("PodmanArgs=--opt custom=extra"), "in:\n{vol}");
}

#[test]
fn healthcheck_start_interval_is_warned_and_omitted() {
	// `start_interval` has no Quadlet/Podman equivalent: it must not emit a
	// `HealthStartupInterval=` (which drives an unrelated, no-op startup
	// healthcheck) and must instead produce a warning.
	let yaml = r#"
services:
  s:
    image: x
    healthcheck:
      test: ["CMD", "true"]
      interval: 5s
      start_interval: 2s
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate(&file, "p");
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(c.contains("HealthInterval=5s"), "in:\n{c}");
	assert!(
		!c.contains("HealthStartupInterval"),
		"start_interval must not emit HealthStartupInterval=; got:\n{c}"
	);
	let joined = out.warnings.join("\n");
	assert!(
		joined.contains("start_interval"),
		"start_interval must warn; got:\n{joined}"
	);
}

#[test]
fn dependency_unit_names_are_sanitized_in_ordering() {
	// A dependency whose compose key sanitizes to a different stem must be
	// referenced by that stem in After=/Requires=, matching the generated unit.
	let yaml = r#"
services:
  web:
    image: nginx
    depends_on:
      - "db:1"
  ? "db:1"
  : { image: postgres }
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate(&file, "proj");
	let web = &unit_named(&out, "proj-web.container").contents;
	assert!(web.contains("After=proj-db_1.service"), "in:\n{web}");
	assert!(web.contains("Requires=proj-db_1.service"), "in:\n{web}");
	// The raw, unsanitized name must not leak into the ordering directives.
	assert!(!web.contains("db:1.service"), "in:\n{web}");
	// The dependency's own unit really is named with the sanitized stem.
	unit_named(&out, "proj-db_1.container");
}

#[test]
fn network_mode_service_maps_to_dot_container() {
	// `network_mode: service:X` reuses a sibling service's netns, which Quadlet
	// expresses as the `Network={X}.container` unit dependency. Generated unit
	// stems are project-prefixed, so the dependency is `p-db.container`.
	let yaml = "services:\n  s:\n    image: x\n    network_mode: \"service:db\"\n";
	let file = parse_str(yaml).unwrap();
	let out = generate(&file, "p");
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(
		c.contains("Network=p-db.container"),
		"service:db must map to Network=p-db.container; got:\n{c}"
	);
}

#[test]
fn network_mode_container_maps_to_join_form() {
	// `network_mode: container:X` joins an *existing* container's netns by id/name
	// via podman's `Network=container:X`; it is not a `.container` unit dependency
	// (that would name a non-existent unit and fail to start).
	let yaml = "services:\n  s:\n    image: x\n    network_mode: \"container:sidecar\"\n";
	let file = parse_str(yaml).unwrap();
	let out = generate(&file, "p");
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(
		c.contains("Network=container:sidecar"),
		"container:sidecar must map to the join form; got:\n{c}"
	);
	assert!(
		!c.contains("sidecar.container"),
		"container: must not emit a .container unit dependency; got:\n{c}"
	);
}

#[test]
fn duplicate_network_aliases_are_emitted_once() {
	// A repeated alias must not produce duplicate `NetworkAlias=` lines, which
	// podman may reject at container create.
	let yaml = "services:\n  s:\n    image: x\n    networks:\n      front:\n        aliases: [dup, dup, uniq]\nnetworks:\n  front:\n";
	let file = parse_str(yaml).unwrap();
	let out = generate(&file, "p");
	let c = &unit_named(&out, "p-s.container").contents;
	assert_eq!(
		c.matches("NetworkAlias=dup").count(),
		1,
		"duplicate alias must be emitted once; got:\n{c}"
	);
	assert_eq!(c.matches("NetworkAlias=uniq").count(), 1, "in:\n{c}");
}

#[test]
fn ipam_options_warn_because_quadlet_cannot_emit_them() {
	// The live engine forwards `ipam.options` via the libpod API, but podman
	// network create / Quadlet expose no key for them, so `generate` warns instead
	// of silently diverging.
	let yaml = "networks:\n  net:\n    ipam:\n      options:\n        foo: bar\nservices:\n  s:\n    image: x\n    networks: [net]\n";
	let file = parse_str(yaml).unwrap();
	let out = generate(&file, "p");
	assert!(
		out.warnings.iter().any(|w| w.contains("ipam.options")),
		"expected an ipam.options warning; got: {:?}",
		out.warnings
	);
}

#[test]
fn network_mode_service_target_is_sanitized() {
	let yaml = "services:\n  s:\n    image: x\n    network_mode: \"service:web:1\"\n";
	let file = parse_str(yaml).unwrap();
	let out = generate(&file, "p");
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(c.contains("Network=p-web_1.container"), "in:\n{c}");
}

#[test]
fn volume_and_network_units_have_no_install_section() {
	let yaml = r#"
services:
  s:
    image: x
networks:
  net:
volumes:
  vol:
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate(&file, "p");
	let net = &unit_named(&out, "p-net.network").contents;
	let vol = &unit_named(&out, "p-vol.volume").contents;
	assert!(
		!net.contains("[Install]") && !net.contains("WantedBy"),
		".network must carry no [Install]; got:\n{net}"
	);
	assert!(
		!vol.contains("[Install]") && !vol.contains("WantedBy"),
		".volume must carry no [Install]; got:\n{vol}"
	);
	// The container unit still carries its [Install].
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(c.contains("[Install]") && c.contains("WantedBy=default.target"));
}

#[test]
fn privileged_maps_to_podman_arg() {
	let yaml = "services:\n  s:\n    image: x\n    privileged: true\n";
	let file = parse_str(yaml).unwrap();
	let out = generate(&file, "p");
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(c.contains("PodmanArgs=--privileged"), "in:\n{c}");
	assert!(
		!out.warnings.iter().any(|w| w.contains("privileged")),
		"privileged must be mapped, not warned; got: {:?}",
		out.warnings
	);
}

#[test]
fn units_carry_podup_ownership_labels() {
	// Every generated unit must carry the same ownership labels the live engine
	// stamps: `podup.project` on all three unit types and `podup.service` on the
	// container, so exported resources are traceable back to their project.
	let yaml = r#"
services:
  web:
    image: nginx:1.27
networks:
  net:
volumes:
  vol:
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate(&file, "proj");

	let c = &unit_named(&out, "proj-web.container").contents;
	assert!(
		c.contains("Label=podup.project=proj"),
		"container missing project ownership label in:\n{c}"
	);
	assert!(
		c.contains("Label=podup.service=web"),
		"container missing service ownership label in:\n{c}"
	);

	let net = &unit_named(&out, "proj-net.network").contents;
	assert!(
		net.contains("Label=podup.project=proj"),
		"network missing project ownership label in:\n{net}"
	);
	// Networks/volumes are project-scoped, not service-scoped: no service label.
	assert!(
		!net.contains("podup.service"),
		"network must not carry a service label in:\n{net}"
	);

	let vol = &unit_named(&out, "proj-vol.volume").contents;
	assert!(
		vol.contains("Label=podup.project=proj"),
		"volume missing project ownership label in:\n{vol}"
	);
	assert!(
		!vol.contains("podup.service"),
		"volume must not carry a service label in:\n{vol}"
	);
}
