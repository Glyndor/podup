//! Resource-limit and `deploy:` rendering tests for the Quadlet export.
//!
//! `internal/backends/quadlet/tests/fields/` carries the rest of the field-mapping
//! coverage. These are pulled out for the same reason `logging.rs` was
//! (#1354): the file was 481 code lines against a 500-line hard limit, which
//! means the pull request that broke it would have been whichever one next
//! added a test here, not the one that made the file long.
//!
//! The cut is by subject rather than by size. Everything here renders a limit
//! or a `deploy:` key into a Podman argument or a systemd directive, which is
//! one question; the rest of `fields.rs` maps compose fields to Quadlet keys.

use crate::parse_str;
use crate::quadlet::generate_at;
use crate::quadlet::tests::assert_argv_has_no_token;
use crate::quadlet::tests::unit_named;

#[test]
fn memory_and_apparmor_render_as_native_keys() {
	// `Memory=` and `AppArmor=` are native [Container] keys in Podman 6.2.0, so
	// they are written as keys and not as `--memory` / `--security-opt` flags.
	// The hostile values of both keys are in `exec/hostile_native_values.rs`.
	let yaml = r#"
services:
  s:
    image: app:1.0
    mem_limit: 512m
    security_opt:
      - "apparmor=my-profile"
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(
		c.contains("Memory=512m\n"),
		"mem_limit must be written as Memory= in:\n{c}"
	);
	assert!(
		c.contains("AppArmor=my-profile\n"),
		"apparmor must be written as AppArmor= in:\n{c}"
	);
	for forbidden in ["--memory", "--security-opt"] {
		assert!(
			!c.contains(forbidden),
			"memory/apparmor must not travel as a podman flag `{forbidden}` in:\n{c}"
		);
	}
}

#[test]
fn cpu_limits_render_as_podman_args() {
	// CPU limits have no native [Container] Quadlet key; they must round-trip
	// through PodmanArgs= rather than being silently dropped.
	let yaml = r#"
services:
  s:
    image: app:1.0
    cpus: "1.5"
    cpuset: "0,1"
    cpu_shares: 512
    cpu_quota: 50000
    cpu_period: 100000
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	// Podman refuses `--cpus` next to `--cpu-quota`/`--cpu-period`
	// (`--cpu-quota and --cpus cannot be set together`); `up` lets `cpu_quota`
	// win, so the unit does too.
	assert!(
		!c.contains("--cpus="),
		"--cpus with a quota fails to start:\n{c}"
	);
	for expected in [
		"PodmanArgs=--cpuset-cpus=\"0,1\"",
		"PodmanArgs=--cpu-shares=512",
		"PodmanArgs=--cpu-quota=50000",
		"PodmanArgs=--cpu-period=100000",
	] {
		assert!(c.contains(expected), "missing `{expected}` in:\n{c}");
	}
}

#[test]
fn deploy_limits_cpus_render_as_podman_args() {
	// `deploy.resources.limits.cpus` is the modern equivalent of `cpus` and
	// must reach the unit too when the top-level `cpus` is absent.
	let yaml = r#"
services:
  s:
    image: app:1.0
    deploy:
      resources:
        limits:
          cpus: "2"
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(
		c.contains("PodmanArgs=--cpus=\"2\""),
		"missing deploy cpus PodmanArgs in:\n{c}"
	);
}

#[test]
fn deploy_limits_pids_maps_to_pids_limit() {
	let yaml = r#"
services:
  s:
    image: x
    deploy:
      resources:
        limits:
          pids: 256
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(c.contains("PidsLimit=256"), "missing PidsLimit in:\n{c}");
}

#[test]
fn deploy_restart_policy_maps_to_systemd() {
	let yaml = r#"
services:
  s:
    image: x
    deploy:
      restart_policy:
        condition: on-failure
        max_attempts: 4
        window: 2m
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(c.contains("Restart=on-failure"), "in:\n{c}");
	assert!(c.contains("StartLimitBurst=4"), "in:\n{c}");
	assert!(c.contains("StartLimitIntervalSec=120"), "in:\n{c}");
}

#[test]
fn deploy_restart_condition_none_maps_to_no() {
	let yaml = "services:\n  s:\n    image: x\n    deploy:\n      restart_policy:\n        condition: none\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	assert!(unit_named(&out, "p-s.container")
		.contents
		.contains("Restart=no"));
}

// ---------------------------------------------------------------------------
// #1734: PodmanArgs= argv safety at the interpolation sites that still use it
// (CPU limits, volume driver options). The keys that moved to native Quadlet
// keys are checked in `exec/hostile_native_values.rs`, because the unit text
// alone does not show the argument the generator builds from them.
//
// `escape_unit_value` returns early for `PodmanArgs` (treating it like
// `Exec`/`Entrypoint`), so the seven `format!()` sites that interpolate a
// compose value into a podman-flag template were emitting raw bytes. A
// hostile compose value containing whitespace would smuggle additional
// `podman run` flags onto the same `PodmanArgs=` line; the unit text reads as
// one innocent line, but systemd's word-splitter and podman's parser see two
// argv elements. The only assertion worth pinning is the argv podman would
// actually build, not the line that produced it.
// ---------------------------------------------------------------------------

#[test]
fn cpus_cannot_smuggle_extra_flags() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    cpus: "1.5 --privileged"
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-web.container").contents;
	assert_argv_has_no_token(c, "--privileged");
}

#[test]
fn cpuset_cannot_smuggle_extra_flags() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    cpuset: "0 --privileged -v /:/hostfs2"
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-web.container").contents;
	assert_argv_has_no_token(c, "--privileged");
	assert_argv_has_no_token(c, "-v");
}

#[test]
fn volume_driver_opt_cannot_smuggle_extra_flags() {
	// The `.volume` unit routes unrecognised driver options through
	// `PodmanArgs=` too, and that site was missed when the container ones
	// were fixed for #1734: the sweep stopped at the `.container` unit.
	// These are `podman volume create` flags rather than `podman run`
	// flags, so the blast radius differs, but the mechanism is identical.
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    volumes:
      - data:/data
volumes:
  data:
    driver_opts:
      arbitrary: "value --label injected=yes"
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let v = &unit_named(&out, "p-data.volume").contents;
	assert_argv_has_no_token(v, "--label");
	assert_argv_has_no_token(v, "injected=yes");
}

#[test]
fn volume_driver_opt_key_cannot_smuggle_extra_flags() {
	// Quoting only the value leaves the same hole on the other side of the
	// `=`. The key is attacker-controlled too: it is a YAML mapping key the
	// compose file chooses.
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    volumes:
      - data:/data
volumes:
  data:
    driver_opts:
      "k --label injected=yes": v
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let v = &unit_named(&out, "p-data.volume").contents;
	assert_argv_has_no_token(v, "--label");
}

#[test]
fn volume_driver_opt_doubles_the_systemd_specifier() {
	// `PodmanArgs=` skips `escape_unit_value`, which is what doubles `%`.
	// An undoubled `%h` is expanded by systemd into the operator's home,
	// so the quoting helper has to do it on this path too, on both halves.
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    volumes:
      - data:/data
volumes:
  data:
    driver_opts:
      "pcent%h": "%h/evil"
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let v = &unit_named(&out, "p-data.volume").contents;
	assert!(
		v.contains("%%h") && !v.contains("=\"%h"),
		"the specifier must be doubled on both halves, in:\n{v}"
	);
}

/// With only `cpu_period` set, `cpus` goes out as the quota `up` derives from it.
#[test]
fn cpus_with_a_period_become_a_quota() {
	let yaml = "services:\n  s:\n    image: app:1.0\n    cpus: \"1.5\"\n    cpu_period: 50000\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(!c.contains("--cpus="), "{c}");
	assert!(c.contains("PodmanArgs=--cpu-quota=150000\n"), "{c}");
	assert!(c.contains("PodmanArgs=--cpu-period=50000\n"), "{c}");
}

#[test]
fn the_start_limits_sit_in_the_unit_section_where_systemd_reads_them() {
	// In `[Service]` systemd ignores `StartLimitIntervalSec=`, so the window of a
	// `max_attempts` was lost; both limits belong between `[Unit]` and `[Container]`.
	let yaml = "services:\n  web:\n    image: x\n    deploy:\n      restart_policy:\n        condition: on-failure\n        max_attempts: 3\n        window: 60s\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-web.container").contents;
	let unit = c.find("[Unit]").expect("a [Unit] section");
	let container = c.find("[Container]").expect("a [Container] section");
	for key in ["StartLimitBurst=3", "StartLimitIntervalSec=60"] {
		let at = c
			.find(key)
			.unwrap_or_else(|| panic!("{key} missing in:\n{c}"));
		assert!(
			unit < at && at < container,
			"{key} is outside [Unit] in:\n{c}"
		);
	}
}
