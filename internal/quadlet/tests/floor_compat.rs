//! Regression guard for the Quadlet export's Podman 5.0 floor.
//!
//! podup promises Podman 5.0 or newer. Quadlet silently drops a whole unit
//! at daemon-reload if any of its keys were added after the running Quadlet
//! release, so a single un-flagged post-5.0 key makes every generated unit
//! with that key fail to start on a 5.0 host. These tests pin that:
//!
//! - `every_emitted_key_is_in_the_5_0_set` walks a compose fixture that
//!   exercises every `Key=` the renderer can produce and asserts each one
//!   is either at the floor (in 5.0 for `[Container]`/`[Pod]`/`[Network]`/
//!   `[Volume]`, in 5.2 for `[Build]`) or registered in `min_podman.rs`.
//!   `PodmanArgs=` / `GlobalArgs=` are exempt (they appear on every unit
//!   type in 5.0).
//! - `every_emitted_key_is_registered` is the coverage companion: a key
//!   rendered but missing from the allowlist fails, so the maintainer
//!   either adds a row or removes the emitter.
//! - `build_unit_emits_a_warning` pins the one-line warning a project
//!   gets when any `.build` unit is written.
//!
//! The companion tests for the source-level inventory and empty-value
//! handling live in `floor_compat_source.rs` and `floor_compat_empty.rs`
//! to keep this file under the per-file code-line cap. The fixture
//! composition is one integration point, the source files are the other:
//! anything the renderer ever emits must be reachable through either the
//! fixtures below or the inventory test, or a regression guard would
//! silently miss it. Add a service here when a new key shows up; add a
//! row in the inventory map when a new file maps to a new unit type.

use super::unit_named;
use crate::parse_str;
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
    image: app:latest
networks:
  front:
volumes:
  data:
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

/// Every `Key=` line in every rendered unit must be either at the floor
/// (in the 5.0 reference for `[Container]`/`[Pod]`/`[Network]`/`[Volume]`,
/// in the 5.2 reference for `[Build]`) or registered in `min_podman.rs`
/// with the right release. This is the regression guard: a new key above
/// the floor must add a row, or this test names it. Both fixtures run, so
/// pod-mode and non-pod-mode keys are both covered (PublishPort= and
/// Network= on `.container` only fire outside pod mode; per-container
/// AddHost= etc. fire only in pod mode).
#[test]
fn every_emitted_key_is_in_the_5_0_set() {
	use crate::quadlet::min_podman::{floor_required, unit_type_from_filename};

	let mut units = render_full();
	units.extend(render_no_pod());
	// #1970: the prebuilt shape used by `podup autostart install --mode
	// quadlet` renders container units with the build's `ImageTag=` and
	// adds `Pull=never`; it must stay floor-compliant the same way the
	// standard shape does.
	units.extend(render_full_prebuilt());
	assert!(!units.is_empty(), "fixture must render at least one unit");
	for unit in &units {
		let unit_type = unit_type_from_filename(&unit.filename);
		for key in crate::quadlet::min_podman::keys_in_unit(&unit.contents) {
			match floor_required(unit_type, &key) {
				// No row, so the key is expected to be at the floor. The
				// `every_emitted_key_is_registered` companion test below
				// catches a key we emit that is missing from the table
				// entirely; this test catches a key whose table row is
				// wrong (a post-5.0 key still being emitted natively).
				None => {}
				Some(v) => panic!(
					"unit {} emits {key}= on {unit_type:?}, which Podman only added in {}; \
					 the floor is 5.0; route the value through PodmanArgs= or update \
					 the table if a recent 5.x release is the new supported floor",
					unit.filename,
					v.fmt()
				),
			}
		}
	}
}

/// Every `Key=` line we render must be registered in the allowlist. A key
/// emitted but missing fails here; the maintainer either adds a row (with
/// the right release) or removes the emitter. Keys at the floor are
/// accepted via the in-line 5.0/5.2 reference sets, even without a table
/// row: the floor covers them implicitly.
#[test]
fn every_emitted_key_is_registered() {
	use crate::quadlet::min_podman::{is_floor_compliant, unit_type_from_filename, UnitType};

	let mut units = render_full();
	units.extend(render_no_pod());
	// #1970: cover the prebuilt shape too, see every_emitted_key_is_in_the_5_0_set.
	units.extend(render_full_prebuilt());
	for unit in &units {
		let unit_type = unit_type_from_filename(&unit.filename);
		for key in crate::quadlet::min_podman::keys_in_unit(&unit.contents) {
			assert!(
				is_floor_compliant(unit_type, &key),
				"unit {} emits {key}= on {unit_type:?}, which is not in the Podman {} reference \
				 for that unit type and which the table does not cover; add a row in \
				 min_podman.rs or remove the emitter",
				unit.filename,
				if matches!(unit_type, UnitType::Build) {
					"5.2"
				} else {
					"5.0"
				}
			);
		}
	}
}

/// The pod-mode pod unit covers the keys that only fire on `.pod`
/// (`AddHost`/`Label`), which the main fixture exercises through the
/// web/db pair but only as `PodmanArgs=`. This pins the conversion
/// explicitly so a regression to the native keys would surface here.
#[test]
fn pod_mode_routes_host_entries_and_label_through_podman_args() {
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
	for forbidden in ["AddHost=", "Label=podup.project="] {
		assert!(
			!pod.contents.contains(forbidden),
			"pod unit must not emit native `{forbidden}`; got:\n{}",
			pod.contents
		);
	}
	assert!(
		pod.contents
			.contains("PodmanArgs=--add-host=\"web:127.0.0.1\""),
		"missing web add-host flag in:\n{}",
		pod.contents
	);
	assert!(
		pod.contents
			.contains("PodmanArgs=--add-host=\"db:127.0.0.1\""),
		"missing db add-host flag in:\n{}",
		pod.contents
	);
	assert!(
		pod.contents
			.contains("PodmanArgs=--label=\"podup.project=demo\""),
		"missing project ownership label flag in:\n{}",
		pod.contents
	);
}

/// A project that emits at least one `.build` unit produces a single
/// per-project warning naming the Podman 5.2.0 floor. The warning goes
/// through `out.warnings`, the same channel every other field-mismatch
/// warning uses.
#[test]
fn build_unit_emits_a_warning() {
	let yaml = r#"
services:
  app:
    build:
      context: ./src
    image: app:1.0
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let joined = out.warnings.join("\n");
	assert!(
		joined.contains(".build") && joined.contains("5.2"),
		"expected a warning mentioning both `.build` and `5.2`; got:\n{joined}"
	);
}

/// A project that does not emit a `.build` unit must not emit the
/// warning. The warning is keyed on the actual rendering, not on the
/// presence of a `build:` field: an inline Dockerfile rejects the unit
/// (and is itself warned) so no `.build` is written and no extra
/// 5.2 warning should fire.
#[test]
fn no_build_unit_means_no_build_floor_warning() {
	let yaml = r#"
services:
  app:
    image: x
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let joined = out.warnings.join("\n");
	assert!(
		!joined.contains("5.2"),
		"no .build unit was written; the 5.2 warning must not fire; got:\n{joined}"
	);
}

/// An inline-Dockerfile build rejects the `.build` unit (the existing
/// path) and therefore does not trigger the 5.2 warning either: the
/// warning is gated on actually writing the unit.
#[test]
fn inline_dockerfile_does_not_trigger_build_floor_warning() {
	let yaml = r#"
services:
  app:
    build:
      dockerfile_inline: |
        FROM alpine
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let joined = out.warnings.join("\n");
	assert!(
		!joined.contains("5.2"),
		"an inline-Dockerfile build does not emit a `.build` unit; the 5.2 \
		 warning must not fire; got:\n{joined}"
	);
}
