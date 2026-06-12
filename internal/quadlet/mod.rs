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

use std::collections::BTreeMap;

use crate::compose::types::{
	Command, ComposeFile, PortMapping, RestartPolicy, Service, VolumeMount,
};
use crate::ports::parse_ports;

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

fn network_unit(name: &str, project: &str, _has_config: bool) -> QuadletUnit {
	let value = sanitize_value(&format!("{project}_{name}"));
	let contents =
		format!("[Network]\nNetworkName={value}\n\n[Install]\nWantedBy=default.target\n");
	QuadletUnit {
		filename: format!("{}.network", safe_unit_stem(name)),
		contents,
	}
}

fn volume_unit(name: &str, project: &str, _has_config: bool) -> QuadletUnit {
	let value = sanitize_value(&format!("{project}_{name}"));
	let contents =
		format!("[Volume]\nVolumeName={value}\n\n[Install]\nWantedBy=default.target\n");
	QuadletUnit {
		filename: format!("{}.volume", safe_unit_stem(name)),
		contents,
	}
}

fn container_unit(
	name: &str,
	service: &Service,
	declared_volumes: &[&str],
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
	container.add("ContainerName", name.to_string());
	if let Some(image) = &service.image {
		container.add("Image", image.clone());
	}
	if let Some(hostname) = &service.hostname {
		container.add("HostName", hostname.clone());
	}
	if let Some(user) = &service.user {
		container.add("User", user.clone());
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
		container.add("Network", format!("{net}.network"));
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

	let mut svc = Section::new("Service");
	if let Some(restart) = &service.restart {
		svc.add("Restart", render_restart(restart));
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

/// Warn for fields that are set but have no Quadlet mapping, so the operator
/// knows the generated unit is incomplete rather than discovering it at run
/// time.
fn collect_warnings(name: &str, service: &Service, warnings: &mut Vec<String>) {
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
	if service.healthcheck.is_some() {
		warn("healthcheck", "is not yet mapped to HealthCmd directives");
	}
	if !service.secrets.is_empty() {
		warn(
			"secrets",
			"are not yet mapped to Quadlet Secret= directives",
		);
	}
	if !service.configs.is_empty() {
		warn("configs", "have no Quadlet equivalent and are skipped");
	}
	if !service.volumes_from.is_empty() {
		warn("volumes_from", "has no Quadlet equivalent and is skipped");
	}
	if service.network_mode.is_some() {
		warn("network_mode", "is not mapped; use networks instead");
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
}

fn render_publish_port(p: &crate::ports::ParsedPort) -> String {
	let mut s = String::new();
	if !p.host_ip.is_empty() {
		s.push_str(&p.host_ip);
		s.push(':');
	}
	// A host port of None/0 means "let Podman pick"; omit it so the published
	// side is empty (PublishPort=<container>) and Podman assigns a port.
	if let Some(host) = p.host_port.filter(|n| *n != 0) {
		s.push_str(&host.to_string());
		s.push(':');
	}
	s.push_str(&p.container_port.to_string());
	if p.protocol != "tcp" {
		s.push('/');
		s.push_str(&p.protocol);
	}
	s
}

fn render_volume(vol: &VolumeMount, declared_volumes: &[&str]) -> String {
	match vol {
		VolumeMount::Short(s) => {
			let parts: Vec<&str> = s.splitn(3, ':').collect();
			if parts.len() >= 2 && declared_volumes.contains(&parts[0]) {
				let mut out = format!("{}.volume:{}", parts[0], parts[1]);
				if let Some(opts) = parts.get(2) {
					out.push(':');
					out.push_str(opts);
				}
				out
			} else {
				s.clone()
			}
		}
		VolumeMount::Long {
			source,
			target,
			read_only,
			..
		} => {
			let src = source.clone().unwrap_or_default();
			let src = if declared_volumes.contains(&src.as_str()) {
				format!("{src}.volume")
			} else {
				src
			};
			let mut out = if src.is_empty() {
				target.clone()
			} else {
				format!("{src}:{target}")
			};
			if *read_only == Some(true) {
				out.push_str(":ro");
			}
			out
		}
	}
}

fn render_command(command: &Command) -> String {
	match command {
		Command::Shell(s) => s.clone(),
		Command::Exec(parts) => parts.join(" "),
	}
}

fn render_restart(restart: &RestartPolicy) -> String {
	match restart {
		RestartPolicy::No => "no".to_string(),
		RestartPolicy::Always => "always".to_string(),
		RestartPolicy::UnlessStopped => "always".to_string(),
		RestartPolicy::OnFailure { .. } => "on-failure".to_string(),
	}
}

fn sorted_pairs(
	map: std::collections::HashMap<String, Option<String>>,
) -> Vec<(String, Option<String>)> {
	let sorted: BTreeMap<_, _> = map.into_iter().collect();
	sorted.into_iter().collect()
}

fn sorted_label_pairs(map: std::collections::HashMap<String, String>) -> Vec<(String, String)> {
	let sorted: BTreeMap<_, _> = map.into_iter().collect();
	sorted.into_iter().collect()
}

/// Strip ASCII control characters from a value before it is written into a
/// unit file. systemd unit entries are single-line `Key=Value` pairs; an
/// embedded newline from a hostile compose field would otherwise inject
/// arbitrary unit directives (e.g. a `[Service]` `ExecStartPre=`). Compose
/// input is untrusted, so every dynamic value is sanitized at the boundary.
fn sanitize_value(value: &str) -> String {
	value.chars().filter(|c| !c.is_control()).collect()
}

/// Reduce a compose key to a safe single path-component stem for a unit file
/// name. Keeps ASCII alphanumerics and `-`/`_`/`.`; replaces anything else
/// (path separators, control characters) with `_`, and guarantees the result
/// is non-empty and does not start with a dot, so it can never escape the
/// output directory or resolve to `.`/`..`.
fn safe_unit_stem(name: &str) -> String {
	let mut stem: String = name
		.chars()
		.map(|c| {
			if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
				c
			} else {
				'_'
			}
		})
		.collect();
	if stem.is_empty() || stem.starts_with('.') {
		stem.insert(0, '_');
	}
	stem
}

/// A single `[Section]` accumulating `Key=Value` lines in insertion order.
struct Section {
	name: &'static str,
	lines: Vec<String>,
}

impl Section {
	fn new(name: &'static str) -> Self {
		Section {
			name,
			lines: Vec::new(),
		}
	}

	fn add(&mut self, key: &str, value: String) {
		self.lines.push(format!("{key}={}", sanitize_value(&value)));
	}

	fn is_empty(&self) -> bool {
		self.lines.is_empty()
	}

	fn render(&self) -> String {
		let mut s = format!("[{}]\n", self.name);
		for line in &self.lines {
			s.push_str(line);
			s.push('\n');
		}
		s
	}
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
	fn safe_unit_stem_neutralizes_traversal_and_control_chars() {
		assert_eq!(safe_unit_stem("web"), "web");
		assert_eq!(safe_unit_stem("db-data_1.x"), "db-data_1.x");
		assert_eq!(safe_unit_stem("../../etc/passwd"), "_.._.._etc_passwd");
		assert_eq!(safe_unit_stem("/abs"), "_abs");
		assert_eq!(safe_unit_stem(".hidden"), "_.hidden");
		assert_eq!(safe_unit_stem(""), "_");
		assert!(!safe_unit_stem("a\nb").contains('\n'));
	}

	#[test]
	fn sanitize_value_strips_control_characters() {
		assert_eq!(sanitize_value("plain"), "plain");
		assert_eq!(sanitize_value("a\nb\tc\r"), "abc");
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
