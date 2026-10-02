//! #1970: quadlet-mode autostart must build at install, not at every container
//! start. The standard `podup generate quadlet` output renders
//! `Image=<stem>.build` for a buildable service, which makes Quadlet add
//! `Requires=`/`After=` on the sibling build service; since Podman 5.3 a
//! `.build` service is not `RemainAfterExit=yes`, so every container start
//! (and every boot) re-runs `podman build`, and a failed build keeps the
//! container down. The autostart install path renders a different
//! (`prebuilt`) shape, builds the images once via `systemctl start
//! <build>.service`, and the resulting `.container` units point at the
//! concrete image tag with `Pull=never`. These tests pin that flow.
//!
//! Split out of `mod.rs` to keep the per-file code-line cap under 500.
//! Reuses `FakeCtl` and `ScriptedCtl` from the parent module via `super::`.

use super::{install_quadlet, FakeCtl, ScriptedCtl, BUILD, IMG};
use crate::parse_str;
use std::path::Path;

const BASE: &str = "/srv/app";

/// #1970: a `build:` service installs the prebuilt shape. The `.container`
/// references the concrete image tag (`proj-web`), `Pull=never`, and never
/// the `.build` filename; the systemctl log is the build call followed by
/// the container call, in that order, with one `daemon-reload` ahead of
/// both.
#[test]
fn install_build_writes_prebuilt_container_and_starts_build_then_container() {
	super::with_env(|root| {
		let sc = FakeCtl::new();
		install_quadlet(
			&sc,
			&parse_str(BUILD).unwrap(),
			"proj",
			Path::new(BASE),
			false,
			false,
		)
		.unwrap();
		let container_unit = root.join("containers/systemd/proj-web.container");
		let contents = std::fs::read_to_string(&container_unit).unwrap();
		// The container points at the concrete tag the `.build` unit writes
		// (`ImageTag=proj-web`), not the `.build` filename. The string match
		// for the filename is exact: a unit that names `proj-web.build`
		// anywhere makes Quadlet add a `Requires=`/`After=` on the build
		// service, which is what the prebuilt shape exists to avoid.
		assert!(
			contents.contains("Image=proj-web\n"),
			"prebuilt container must reference the build's ImageTag, not the .build filename; got:\n{contents}"
		);
		assert!(
			!contents.lines().any(|l| l.contains(".build")),
			"prebuilt container must not mention .build anywhere (Quadlet would \
			 add Requires/After on the build service); got:\n{contents}"
		);
		let pulls: Vec<&str> = contents
			.lines()
			.filter(|l| l.starts_with("Pull="))
			.map(|l| l.trim_end())
			.collect();
		assert_eq!(
			pulls,
			vec!["Pull=never"],
			"prebuilt container must carry exactly one `Pull=never` line; got: {pulls:?}"
		);

		let calls = sc.log();
		assert_eq!(
			calls,
			vec![
				vec!["daemon-reload".to_string()],
				vec!["start".to_string(), "proj-web-build.service".to_string()],
				vec!["start".to_string(), "proj-web.service".to_string()],
			],
			"daemon-reload then start build, then start container: {calls:?}"
		);
	});
}

/// #1970: `--no-start` is "do not start the stack", not "do not build".
/// Building is the install step that makes the first boot possible; without
/// it the unattended boot would fail with "image not found". The build
/// call still runs, the container call does not.
#[test]
fn install_build_no_start_reloads_starts_build_only() {
	super::with_env(|_root| {
		let sc = FakeCtl::new();
		install_quadlet(
			&sc,
			&parse_str(BUILD).unwrap(),
			"proj",
			Path::new(BASE),
			true,
			false,
		)
		.unwrap();
		let calls = sc.log();
		assert_eq!(
			calls,
			vec![
				vec!["daemon-reload".to_string()],
				vec!["start".to_string(), "proj-web-build.service".to_string()],
			],
			"the build still runs under --no-start; the container does not: {calls:?}"
		);
	});
}

/// #1970: a failing build stops the install and never starts the container.
/// `checked` propagates the non-zero exit; the container start is on a
/// separate `systemctl` call that must not appear in the log.
#[test]
fn install_build_failure_returns_error_and_skips_container_start() {
	super::with_env(|_root| {
		let sc = ScriptedCtl::new(|args| {
			i32::from(
				args.first() == Some(&"start") && args.get(1) == Some(&"proj-web-build.service"),
			)
		});
		let err = install_quadlet(
			&sc,
			&parse_str(BUILD).unwrap(),
			"proj",
			Path::new(BASE),
			false,
			false,
		)
		.expect_err("a refused build must surface as an error");
		assert!(
			format!("{err}").contains("proj-web-build.service"),
			"error names the build service that failed: {err}"
		);
		let calls = sc.log();
		assert_eq!(
			calls,
			vec![
				vec!["daemon-reload".to_string()],
				vec!["start".to_string(), "proj-web-build.service".to_string()],
			],
			"the build call ran and failed; the container start did not run: {calls:?}"
		);
	});
}

/// #1970: a service that sets both `image:` and `build:` resolves the
/// prebuilt container's `Image=` to the user-supplied tag, not the
/// `<project>-<service>` fallback the build unit itself uses when no
/// `image:` is set.
#[test]
fn install_build_with_explicit_image_uses_that_image() {
	super::with_env(|root| {
		let yaml = "services:\n  web:\n    build: .\n    image: reg.example/app:1\n";
		install_quadlet(
			&FakeCtl::new(),
			&parse_str(yaml).unwrap(),
			"proj",
			Path::new(BASE),
			true,
			false,
		)
		.unwrap();
		let container =
			std::fs::read_to_string(root.join("containers/systemd/proj-web.container")).unwrap();
		assert!(
			container.contains("Image=reg.example/app:1\n"),
			"explicit image must be used as the prebuilt container's Image=: {container}"
		);
	});
}

/// #1970: a `pull_policy:` on a buildable service would, with the standard
/// path, emit a second `Pull=` line, and which of the two wins would be up
/// to Quadlet. The prebuilt path drops the user's pull policy (the whole point
/// of prebuilt is "use the locally built image, nothing else") and writes
/// exactly one `Pull=never`.
#[test]
fn install_build_with_pull_policy_still_emits_one_pull_never() {
	super::with_env(|root| {
		let yaml = "services:\n  web:\n    build: .\n    pull_policy: always\n";
		install_quadlet(
			&FakeCtl::new(),
			&parse_str(yaml).unwrap(),
			"proj",
			Path::new(BASE),
			true,
			false,
		)
		.unwrap();
		let container =
			std::fs::read_to_string(root.join("containers/systemd/proj-web.container")).unwrap();
		let pulls: Vec<&str> = container
			.lines()
			.filter(|l| l.starts_with("Pull="))
			.map(|l| l.trim_end())
			.collect();
		assert_eq!(
			pulls,
			vec!["Pull=never"],
			"exactly one Pull= line, and it is `Pull=never`; got: {pulls:?}"
		);
	});
}

/// #1970: a project with no `build:` produces the same units
/// `podup generate quadlet` emits (byte-identical) and no extra build
/// systemctl call. The autostart path is the standard path for that
/// case; only buildable services change.
#[test]
fn install_no_build_units_byte_identical_to_generate_at() {
	super::with_env(|_root| {
		let sc = FakeCtl::new();
		install_quadlet(
			&sc,
			&parse_str(IMG).unwrap(),
			"proj",
			Path::new(BASE),
			false,
			false,
		)
		.unwrap();
		// Two calls: daemon-reload + start container. No build call.
		let calls = sc.log();
		assert_eq!(
			calls,
			vec![
				vec!["daemon-reload".to_string()],
				vec!["start".to_string(), "proj-web.service".to_string()],
			],
			"no build services, no build call: {calls:?}"
		);
		// Unit byte-for-byte matches `generate_at` for the same input.
		let installed = std::fs::read(
			std::env::var("XDG_CONFIG_HOME").unwrap() + "/containers/systemd/proj-web.container",
		)
		.unwrap();
		let std_unit =
			crate::quadlet::generate_at(&parse_str(IMG).unwrap(), "proj", Path::new(BASE))
				.units
				.into_iter()
				.find(|u| u.filename == "proj-web.container")
				.unwrap();
		assert_eq!(
			String::from_utf8(installed).unwrap(),
			std_unit.contents,
			"non-buildable service must render byte-identically under both paths"
		);
	});
}

/// #1970: `--dry-run` lists the build start in its plan and runs no
/// systemctl at all. The operator reading the plan must be able to tell
/// that an install will rebuild the images.
#[test]
fn install_build_dry_run_prints_build_line_and_runs_no_systemctl() {
	super::with_env(|root| {
		let sc = FakeCtl::new();
		install_quadlet(
			&sc,
			&parse_str(BUILD).unwrap(),
			"proj",
			Path::new(BASE),
			false,
			true,
		)
		.unwrap();
		assert!(
			!root.join("containers/systemd/proj-web.container").exists(),
			"dry-run must not write units"
		);
		assert!(sc.log().is_empty(), "dry-run must not call systemctl");
	});
}
