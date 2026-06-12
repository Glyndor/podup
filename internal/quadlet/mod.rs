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

	for (name, cfg) in &file.networks {
		out.units.push(network_unit(name, project, cfg.is_some()));
	}
	for (name, cfg) in &file.volumes {
		out.units.push(volume_unit(name, project, cfg.is_some()));
	}

	let declared_volumes: Vec<&str> = file.volumes.keys().map(String::as_str).collect();
	for (name, service) in &file.services {
		out.units.push(container_unit(
			name,
			service,
			&declared_volumes,
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
		assert!(c.contains("User=1000:1000"));
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
    healthcheck:
      test: ["CMD", "true"]
    network_mode: host
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
			"healthcheck",
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
}
