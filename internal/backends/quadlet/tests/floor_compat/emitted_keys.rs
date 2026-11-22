//! Regression guard for the keys the Quadlet export writes.
//!
//! podup requires Podman 6.2.0 or newer. Quadlet drops a whole unit at
//! daemon-reload if any of its keys is unknown to the running Quadlet
//! release, so a key newer than the floor must never reach a unit. These tests
//! pin that:
//!
//! - `every_emitted_key_is_a_floor_key` walks a compose fixture that exercises
//!   every `Key=` the renderer can produce and asserts each one is in the
//!   allowlist of the keys Podman 6.2.0's Quadlet reads (`min_podman.rs`), so a
//!   key added after the floor names itself. `PodmanArgs=` / `GlobalArgs=` are
//!   exempt (they exist on every unit type).
//! - `the_generator_accepts_every_emitted_key` runs the same units through the
//!   real Quadlet generator of this machine and fails on any unsupported key.
//! - `pod_mode_writes_host_entries_and_label_as_native_keys` pins the
//!   `[Pod]` keys.
//! - `build_unit_carries_no_podman_version_warning` pins that writing a
//!   `.build` unit does not warn about the Podman release.
//!
//! The companion tests for the source-level inventory and empty-value
//! handling live in `source_inventory.rs` and `empty_values.rs`
//! to keep this file under the per-file code-line cap. The fixture
//! composition is one integration point, the source files are the other:
//! anything the renderer ever emits must be reachable through either the
//! fixtures below or the inventory test, or a regression guard would
//! silently miss it. Add a service here when a new key shows up; add a
//! row in the inventory map when a new file maps to a new unit type.

use crate::parse_str;
use crate::quadlet::tests::unit_named;
use crate::quadlet::{generate_at, generate_for_autostart, QuadletUnit};

/// Run `generate` on a fixture that exercises every key we want the floor
/// test to see. New emitters must extend this list; the floor test only
/// covers the keys the fixture actually renders.
fn fixture_full() -> &'static str {
	r#"
x-podman-pod: true
services:
  web:
    image: nginx
    hostname: web-host
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
      - "127.0.0.1:8080:80"
      - "5432:5432"
    extra_hosts:
      - "db:10.0.0.2"
    dns:
      - 1.1.1.1
    dns_search:
      - example.com
    dns_opt:
      - ndots:1
    sysctls:
      net.core.somaxconn: "1024"
    ulimits:
      nofile:
        soft: 1024
        hard: 2048
    devices:
      - /dev/fuse
    shm_size: 64m
    mem_limit: 512m
    pids_limit: 100
    userns_mode: keep-id
    stop_signal: SIGTERM
    stop_grace_period: 30s
    group_add:
      - audio
    security_opt:
      - "no-new-privileges:true"
      - "seccomp=/etc/seccomp.json"
      - "label=type:container_t"
      - "apparmor=my-profile"
    logging:
      driver: journald
      options:
        tag: mytag
    pull_policy: always
    tmpfs:
      - /run
    env_file:
      - ./app.env
    annotations:
      run.oci.keep: "1"
    container_name: custom
    restart: "on-failure:5"
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost"]
      interval: 5s
      retries: 3
    x-podman-autoupdate: registry
    networks:
      front:
        aliases:
          - web-alias
          - dup
          - dup
  db:
    image: postgres
    ports:
      - "5432:5432"
  app:
    build:
      context: ./src
      dockerfile: Dockerfile.app
      target: runtime
      args:
        VERSION: "1.0"
      pull: true
      platforms:
        - linux/arm64/v8
      secrets:
        - tok
    image: app:latest
networks:
  front:
volumes:
  data:
secrets:
  tok:
    file: token.txt
"#
}

fn render_full() -> Vec<QuadletUnit> {
	let yaml = fixture_full();
	let file = parse_str(yaml).unwrap();
	generate_at(&file, "p", std::path::Path::new("/srv/app")).units
}

/// A non-pod-mode fixture. The pod-mode `fixture_full` covers pod-mode
/// behaviour (each `.container` drops `PublishPort=`/`Network=`, the project
/// `.pod` carries them); outside pod mode every container carries its own
/// `PublishPort=` and `Network=` lines, so the floor test must also see
/// those. The networks and volumes carry their driver / IPAM / driver-opts
/// configuration so the `[Network]` and `[Volume]` emitters are exercised
/// beyond the labels-only path the pod-mode fixture covers.
fn fixture_no_pod() -> &'static str {
	r#"
services:
  web:
    image: nginx
    ports:
      - "127.0.0.1:8080:80"
      - "5432:5432"
    networks:
      - front
      - back
  db:
    image: postgres
    ports:
      - "5432:5432"
    networks:
      - back
networks:
  front:
    driver: bridge
    driver_opts:
      mtu: "1500"
    ipam:
      driver: host-local
      config:
        - subnet: 10.7.0.0/16
          gateway: 10.7.0.1
  back:
    driver: bridge
    enable_ipv6: true
volumes:
  data:
    driver: local
    driver_opts:
      type: nfs
      device: ":/exports"
      o: "addr=10.0.0.1,rw"
"#
}

fn render_no_pod() -> Vec<QuadletUnit> {
	let yaml = fixture_no_pod();
	let file = parse_str(yaml).unwrap();
	generate_at(&file, "np", std::path::Path::new("/srv/app")).units
}

/// #1970: render the prebuilt shape `generate_for_autostart` produces for
/// the same input `fixture_full` drives through the standard path. The
/// container unit shifts from `<stem>.build` to the build's `ImageTag=` and
/// picks up `Pull=never`; everything else renders the same. The floor
/// tests below cover both shapes so a key that the prebuilt shape needs
/// (and the standard path does not) cannot slip past unrecorded.
fn render_full_prebuilt() -> Vec<QuadletUnit> {
	let yaml = fixture_full();
	let file = parse_str(yaml).unwrap();
	generate_for_autostart(&file, "p", std::path::Path::new("/srv/app")).units
}

/// Every unit the guards below look at: the standard shape in pod and non-pod
/// mode, plus the prebuilt shape `podup autostart install --mode quadlet`
/// renders (#1970), which gives container units the build's `ImageTag=` and
/// adds `Pull=never`.
fn all_rendered_units() -> Vec<QuadletUnit> {
	let mut units = render_full();
	units.extend(render_no_pod());
	units.extend(render_full_prebuilt());
	assert!(!units.is_empty(), "fixture must render at least one unit");
	units
}

/// Every `Key=` line in every rendered unit must be a key the Quadlet of the
/// floor (Podman 6.2.0) reads. Both fixtures run, so pod-mode and non-pod-mode
/// keys are both covered (PublishPort= and Network= on `.container` only fire
/// outside pod mode; per-container AddHost= etc. fire only in pod mode).
#[test]
fn every_emitted_key_is_a_floor_key() {
	use crate::quadlet::min_podman::{is_at_floor, keys_in_unit, unit_type_from_filename};

	for unit in &all_rendered_units() {
		let unit_type = unit_type_from_filename(&unit.filename);
		for key in keys_in_unit(&unit.contents) {
			assert!(
				is_at_floor(unit_type, &key),
				"unit {} emits {key}= on {unit_type:?}, which Podman 6.2.0's Quadlet does not \
				 read; add it to the allowlist in min_podman.rs only after the generator \
				 accepts it, or write the value as a PodmanArgs= flag",
				unit.filename,
			);
		}
	}
}

/// The allowlist is a claim about the Quadlet of the floor; this checks it
/// against the Quadlet of this machine. The generator reports each key it
/// does not know as `unsupported key`. The test returns early where the
/// generator is not installed.
#[test]
fn the_generator_accepts_every_emitted_key() {
	let Some(generator) = ["/usr/libexec/podman/quadlet", "/usr/lib/podman/quadlet"]
		.iter()
		.map(std::path::PathBuf::from)
		.find(|p| p.is_file())
	else {
		return;
	};
	for (shape, units) in [
		("standard", render_full()),
		("no pod", render_no_pod()),
		("prebuilt", render_full_prebuilt()),
	] {
		let dir = tempfile::tempdir().unwrap();
		crate::quadlet::write_units(dir.path(), &units).unwrap();
		let run = std::process::Command::new(&generator)
			.args(["-dryrun", "-user"])
			.env("QUADLET_UNIT_DIRS", dir.path())
			.output()
			.unwrap();
		let stderr = String::from_utf8_lossy(&run.stderr);
		assert!(
			!stderr.contains("unsupported key"),
			"the Quadlet generator rejects a key of the {shape} shape:\n{stderr}"
		);
	}
}

/// The pod unit carries the host entries and the ownership label as native
/// keys, and the exit policy as `ExitPolicy=`.
#[test]
fn pod_mode_writes_host_entries_and_label_as_native_keys() {
	let yaml = r#"
x-podman-pod: true
services:
  web:
    image: nginx
    ports:
      - "8080:80"
  db:
    image: postgres
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "demo", std::path::Path::new("/srv/app"));
	let pod = unit_named(&out, "demo.pod");
	for wanted in [
		"AddHost=web:127.0.0.1\n",
		"AddHost=db:127.0.0.1\n",
		"Label=podup.project=demo\n",
		"ExitPolicy=continue\n",
		"PodmanArgs=--share=net\n",
	] {
		assert!(
			pod.contents.contains(wanted),
			"pod unit must carry `{wanted}`; got:\n{}",
			pod.contents
		);
	}
	for forbidden in ["--add-host", "--label", "--exit-policy"] {
		assert!(
			!pod.contents.contains(forbidden),
			"pod unit must not route `{forbidden}` through PodmanArgs=; got:\n{}",
			pod.contents
		);
	}
}

/// Writing a `.build` unit adds no warning about the Podman release: the
/// unit type arrived in 5.2.0 and the floor is 6.2.0, so every supported
/// Podman reads it. The project below does write one.
#[test]
fn build_unit_carries_no_podman_version_warning() {
	let yaml = r#"
services:
  app:
    build:
      context: ./src
    image: app:1.0
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	assert!(
		out.units.iter().any(|u| u.filename.ends_with(".build")),
		"the fixture must write a `.build` unit; got {:?}",
		out.units.iter().map(|u| &u.filename).collect::<Vec<_>>()
	);
	let joined = out.warnings.join("\n");
	assert!(
		!joined.contains("5.2") && !joined.contains("Podman"),
		"a `.build` unit needs no Podman release warning; got:\n{joined}"
	);
}
