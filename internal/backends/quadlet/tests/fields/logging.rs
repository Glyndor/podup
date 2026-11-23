//! Log-rotation rendering tests for the Quadlet export.
//!
//! `internal/backends/quadlet/tests/fields/` carries the rest of the field-mapping
//! coverage; the log-rotation tests are pulled out into this file because
//! `fields.rs` was already near the per-file line cap and these two tests
//! alone were enough to push it over (#1354).

use crate::parse_str;
use crate::quadlet::generate_at;
use crate::quadlet::tests::unit_named;

/// A service with no `logging:` block gets the rotation default so the
/// generated quadlet units ship the same rotation policy that `up` would
/// apply at runtime (#1354). Without this, `generate quadlet` would emit a
/// unit that runs without rotation, diverging from `up` behaviour.
///
/// The cap is rendered as a byte count rather than `10m` since #1417 moved
/// it into libpod's typed `size` field. Measured on Podman 6.2.0
/// (2026-11-22): `--log-opt max-size=10485760` stores a `10.49MB` cap (10 MiB,
/// what `up` sets), where `max-size=10m` stores `10MB`, so the byte count is
/// the faithful spelling. `max-file` is deliberately absent: `podman create
/// --log-opt max-file=3` is accepted and stores no configuration, so emitting
/// it promised a rotation nothing performed.
///
/// Both the default rotation and the user options are written as `LogOpt=`.
#[test]
fn logging_default_is_emitted_when_logging_block_is_absent() {
	let yaml = r#"
services:
  s:
    image: x
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(
		c.contains("LogDriver=k8s-file"),
		"missing default LogDriver in:\n{c}"
	);
	assert!(
		c.contains("LogOpt=max-size=10485760\n"),
		"missing default max-size LogOpt in:\n{c}"
	);
	assert!(
		!c.contains("max-file"),
		"max-file is honoured by neither the API nor the CLI; emitting it \
		 promises a rotation nothing performs:\n{c}"
	);
}

/// An explicit `logging:` block overrides the default: the rendered unit
/// carries the user's driver and options, not the default (#1354).
#[test]
fn logging_user_override_replaces_the_default() {
	let yaml = r#"
services:
  s:
    image: x
    logging:
      driver: journald
      options:
        tag: mytag
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(
		c.contains("LogDriver=journald"),
		"user override not applied:\n{c}"
	);
	assert!(
		c.contains("LogOpt=tag=mytag\n"),
		"user options not applied:\n{c}"
	);
	// The user override replaces the whole `options:` set, so no default
	// `max-size` survives.
	assert!(
		!c.contains("max-size"),
		"default leaked through override:\n{c}"
	);
}

/// `logging.options.path` is the log file's place for `up` through the typed
/// `path` field; the unit gets the same through `--log-opt path=`, which the
/// CLI turns into that field. A path under a driver that writes no file is
/// left out, as `up` leaves it out.
#[test]
fn logging_path_is_emitted_under_a_file_driver_and_left_out_under_journald() {
	let render = |driver: &str| {
		let yaml = format!(
			"services:\n  s:\n    image: x\n    logging:\n      driver: {driver}\n      options:\n        path: /var/log/s.log\n"
		);
		let file = parse_str(&yaml).unwrap();
		let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
		unit_named(&out, "p-s.container").contents.clone()
	};
	let file_driver = render("k8s-file");
	assert!(
		file_driver.contains("LogOpt=path=/var/log/s.log\n"),
		"{file_driver}"
	);
	let journald = render("journald");
	assert!(!journald.contains("path="), "{journald}");
}
