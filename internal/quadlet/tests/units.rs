use super::unit_named;
use crate::parse_str;
use crate::quadlet::{generate, QuadletOutput, QuadletUnit};

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
	let net = &unit_named(&out, "net.network").contents;
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
	let vol = &unit_named(&out, "vol.volume").contents;
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
