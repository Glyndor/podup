use crate::parse_str;
use crate::quadlet::generate_at;
use crate::quadlet::tests::unit_named;

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
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));

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
fn bind_path_volume_is_passed_through() {
	let yaml = r#"
services:
  web:
    image: nginx
    volumes:
      - ./html:/usr/share/nginx/html:ro
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));
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
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "proj-db.container").contents;
	assert!(c.contains("Volume=proj-pgdata.volume:/var/lib/postgresql/data:ro"));
}

#[test]
fn newline_in_value_cannot_inject_unit_directives() {
	// An environment value carrying a newline plus a forged directive must
	// be flattened to a single line, not injected as a new unit entry.
	let yaml =
		"services:\n  web:\n    image: x\n    environment:\n      EVIL: \"a\\nExecStartPre=/bin/rm -rf /\"\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "proj-web.container").contents;
	assert!(
		!c.lines().any(|l| l.starts_with("ExecStartPre")),
		"a newline in a value must not inject a directive line:\n{c}"
	);
}

/// Implicit dependencies show up in the Quadlet `[Unit]` section: a service
/// that joins another through `volumes_from: [data]` must carry
/// `After=`/`Requires=` for the data unit, so systemd orders creation the
/// way docker compose does and the volume-from reference never names a
/// container that does not exist yet.
#[test]
fn volumes_from_emits_after_and_requires_in_unit_section() {
	let yaml = "services:\n  data:\n    image: x\n  ro:\n    image: x\n    volumes_from: [data]\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "proj-ro.container").contents;
	let unit_section = c
		.split("\n[Container]")
		.next()
		.expect("container unit has [Unit] section");
	assert!(
		unit_section.contains("After=proj-data.service"),
		"ro unit must order itself after the data unit; got:\n{c}"
	);
	assert!(
		unit_section.contains("Requires=proj-data.service"),
		"ro unit must require the data unit; got:\n{c}"
	);
}

#[test]
fn privileged_maps_to_podman_arg() {
	let yaml = "services:\n  s:\n    image: x\n    privileged: true\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(c.contains("PodmanArgs=--privileged"), "in:\n{c}");
	assert!(
		!out.warnings.iter().any(|w| w.contains("privileged")),
		"privileged must be mapped, not warned; got: {:?}",
		out.warnings
	);
}
