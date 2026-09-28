//! Tests for the cross-mode refusals: service and start modes cannot coexist
//! for the same project; re-installing the same mode still rewrites the unit;
//! the unknown-shape case refuses rather than overwrites.
//!
//! Each scenario writes the pre-existing unit through the real installer (the
//! same one `conflict::refuse_if_other_single_unit_mode` guards), so the file
//! on disk matches what an actual install would have produced, then attempts
//! the other install and asserts the refusal.

use std::path::PathBuf;

use super::conflict::{classify_exec_start, DetectedSingleUnitMode};
use super::start::StartUnitOpts;
use super::tests::{opts, opts_with_interval, with_env, FakeCtl};
use super::*;

fn start_opts(project: &str, container: &str) -> StartUnitOpts {
	StartUnitOpts::new(
		PathBuf::from("/usr/bin/podman"),
		project.to_string(),
		container.to_string(),
	)
}

/// Case 1: service installed, `install_start` is refused. The pre-existing
/// service unit was written by the real installer (so the shape is exactly
/// what the refusal will encounter in the field), and the bytes are compared
/// before and after the refused attempt to prove no silent overwrite.
#[test]
fn install_start_refuses_over_a_service_install() {
	with_env(|root| {
		let sc = FakeCtl::new();
		install(&sc, &opts(root, "app", false, false)).unwrap();
		let path = root.join("systemd/user/podup-app.service");
		let before = std::fs::read(&path).expect("service unit written");

		let sc2 = FakeCtl::new();
		let err = install_start(&sc2, &start_opts("app", "app-web-1"), false, false)
			.expect_err("install_start must refuse over an existing service unit");
		assert!(matches!(err, ComposeError::Autostart(_)), "got {err:?}");
		let msg = err.to_string();
		assert!(
			msg.contains("podup autostart uninstall"),
			"the message must name the recovery command: {msg}"
		);

		let after = std::fs::read(&path).expect("unit still on disk");
		assert_eq!(before, after, "refused install must not touch the file");
		assert!(
			sc2.systemctl_log().is_empty(),
			"no systemctl call before refusal"
		);
	});
}

/// Case 2: start installed, `install` (service) is refused.
#[test]
fn install_refuses_over_a_start_install() {
	with_env(|root| {
		let sc = FakeCtl::new();
		install_start(&sc, &start_opts("app", "app-web-1"), false, false).unwrap();
		let path = root.join("systemd/user/podup-app.service");
		let before = std::fs::read(&path).expect("start unit written");

		let sc2 = FakeCtl::new();
		let err = install(&sc2, &opts(root, "app", false, false))
			.expect_err("install must refuse over an existing start unit");
		assert!(matches!(err, ComposeError::Autostart(_)), "got {err:?}");
		assert!(
			err.to_string().contains("podup autostart uninstall"),
			"the message must name the recovery command: {err}"
		);

		let after = std::fs::read(&path).expect("unit still on disk");
		assert_eq!(before, after, "refused install must not touch the file");
		assert!(
			sc2.systemctl_log().is_empty(),
			"no systemctl call before refusal"
		);
	});
}

/// Case 3: a service install with `--auto-update` writes the main unit and
/// the timer pair. Refusing `install_start` must leave all three files alone.
#[test]
fn install_start_refuses_over_a_service_install_with_auto_update() {
	with_env(|root| {
		let sc = FakeCtl::new();
		install(&sc, &opts_with_interval(root, "app", false, false, "daily")).unwrap();
		let main = root.join("systemd/user/podup-app.service");
		let oneshot = root.join("systemd/user/podup-app-update.service");
		let timer = root.join("systemd/user/podup-app-update.timer");
		let before_main = std::fs::read(&main).unwrap();
		let before_oneshot = std::fs::read(&oneshot).unwrap();
		let before_timer = std::fs::read(&timer).unwrap();

		let sc2 = FakeCtl::new();
		let _ = install_start(&sc2, &start_opts("app", "app-web-1"), false, false)
			.expect_err("must refuse");

		assert_eq!(std::fs::read(&main).unwrap(), before_main, "main untouched");
		assert_eq!(
			std::fs::read(&oneshot).unwrap(),
			before_oneshot,
			"oneshot untouched"
		);
		assert_eq!(
			std::fs::read(&timer).unwrap(),
			before_timer,
			"timer untouched"
		);
	});
}

/// Case 4: re-installing the SAME service mode overwrites the unit in place.
/// The new compose path must be reflected in the file (so this is the
/// documented upgrade path, not a silent no-op).
#[test]
fn install_overwrites_a_same_mode_install_with_different_compose_path() {
	with_env(|root| {
		let sc = FakeCtl::new();
		install(&sc, &opts(root, "app", false, false)).unwrap();
		let path = root.join("systemd/user/podup-app.service");
		let before = std::fs::read(&path).unwrap();

		let mut other_opts = opts(root, "app", false, false);
		other_opts.unit.compose_files = vec![root.join("docker-compose.v2.yml")];
		let sc2 = FakeCtl::new();
		install(&sc2, &other_opts).expect("same-mode re-install is the upgrade path");

		let after = std::fs::read(&path).unwrap();
		assert_ne!(
			before, after,
			"the upgrade must rewrite the unit, not leave it unchanged"
		);
		let body = String::from_utf8_lossy(&after);
		assert!(
			body.contains("docker-compose.v2.yml"),
			"the new compose path must be in the rewritten unit:\n{body}"
		);
	});
}

/// Case 5: re-installing start mode over itself succeeds.
#[test]
fn install_start_overwrites_a_same_mode_install() {
	with_env(|root| {
		let sc = FakeCtl::new();
		install_start(&sc, &start_opts("app", "app-web-1"), false, false).unwrap();
		let path = root.join("systemd/user/podup-app.service");
		let before = std::fs::read(&path).unwrap();

		let mut different = start_opts("app", "app-web-1");
		different = different.with_stop_grace_secs(Some(42));
		let sc2 = FakeCtl::new();
		install_start(&sc2, &different, false, false)
			.expect("same-mode re-install is the upgrade path");

		let after = std::fs::read(&path).unwrap();
		assert_ne!(
			before, after,
			"the second start-mode install must rewrite the unit, not leave it untouched"
		);
		let body = String::from_utf8_lossy(&after);
		assert!(
			body.contains("TimeoutStopSec=72"),
			"the new stop grace must be in the rewritten unit:\n{body}"
		);
	});
}

/// Case 6: a unit whose `ExecStart=` does not match either shape. Both
/// installers refuse; the file is unchanged.
#[test]
fn both_installers_refuse_over_a_unit_with_an_unrecognised_exec_start() {
	with_env(|root| {
		let dir = root.join("systemd/user");
		std::fs::create_dir_all(&dir).unwrap();
		let path = dir.join("podup-app.service");
		let body = "\
[Unit]
Description=hand-written

[Service]
Type=oneshot
ExecStart=/bin/sleep 60
";
		std::fs::write(&path, body).unwrap();
		let before = std::fs::read(&path).unwrap();

		let sc1 = FakeCtl::new();
		let err1 = install(&sc1, &opts(root, "app", false, false))
			.expect_err("install must refuse over an unknown-shape unit");
		assert!(matches!(err1, ComposeError::Autostart(_)), "got {err1:?}");
		assert!(
			err1.to_string().contains("podup autostart uninstall"),
			"refusal must name the recovery command: {err1}"
		);

		let sc2 = FakeCtl::new();
		let err2 = install_start(&sc2, &start_opts("app", "app-web-1"), false, false)
			.expect_err("install_start must refuse over an unknown-shape unit");
		assert!(matches!(err2, ComposeError::Autostart(_)), "got {err2:?}");

		let after = std::fs::read(&path).unwrap();
		assert_eq!(before, after, "unknown-shape unit must not be overwritten");
		assert!(sc1.systemctl_log().is_empty());
		assert!(sc2.systemctl_log().is_empty());
	});
}

/// Case 7: token-based classification of `ExecStart=` lines, so a project or
/// container named `start` or `up` cannot be misread.
///
/// service line whose project is literally `start`: ends with
/// `up -d --no-build --pull never`, classified as service.
/// start line whose container is literally `up`: `<podman> start up`,
/// classified as start.
/// Path that contains the word `up` as a directory component: not matched.
/// Line with the substring `start` in the trailing args: not matched.
#[test]
fn parser_uses_tokens_not_substrings() {
	let service_with_start_project = "ExecStart=/usr/local/bin/podup -f /srv/start/docker-compose.yml -p start up -d --no-build --pull never";
	assert_eq!(
		classify_exec_start(service_with_start_project),
		DetectedSingleUnitMode::Service,
		"a service unit whose project is 'start' must still classify as service"
	);
	let start_with_up_container = "ExecStart=/usr/bin/podman start up";
	assert_eq!(
		classify_exec_start(start_with_up_container),
		DetectedSingleUnitMode::Start,
		"a start unit whose container is 'up' must still classify as start"
	);
	// Pathological: a path containing `up` in the middle. The suffix is `up`
	// alone, not `up -d`, so it is unknown, not misread as service.
	let path_contains_up = "ExecStart=/srv/up/bin/foo";
	assert_eq!(
		classify_exec_start(path_contains_up),
		DetectedSingleUnitMode::Unknown,
		"a trailing 'up' without the '-d' pair is not service mode"
	);
	// Service mode's older shape: `up -d` with no `--no-build --pull never`
	// trailing args. Must still be recognised as service.
	let older_service =
		"ExecStart=/usr/local/bin/podup -f /srv/app/docker-compose.yml -p app up -d";
	assert_eq!(
		classify_exec_start(older_service),
		DetectedSingleUnitMode::Service,
		"older releases wrote 'up -d' without --no-build --pull never; the refusal must recognise them"
	);
	// Trailing args end in something that looks like the start token but
	// isn't, so a unit whose last token is the literal word `start` is not
	// mistaken for start mode (which needs exactly three tokens, with
	// `start` as the second).
	let almost_start = "ExecStart=/usr/bin/podman up -d start";
	assert_ne!(
		classify_exec_start(almost_start),
		DetectedSingleUnitMode::Start,
		"three tokens are required for start mode, with 'start' as the second"
	);
}

/// Case 8: `--dry-run` over the other mode refuses too. Whatever flag or
/// argument the installers take for dry-run (`install` via
/// `InstallOptions::with_dry_run`, `install_start` via its own boolean),
/// both must report the refusal before any unit is written.
#[test]
fn dry_run_refuses_over_the_other_mode_too() {
	with_env(|root| {
		// Install service for real first, then dry-run start mode.
		let sc = FakeCtl::new();
		install(&sc, &opts(root, "app", false, false)).unwrap();
		let path = root.join("systemd/user/podup-app.service");
		let before = std::fs::read(&path).unwrap();

		let sc2 = FakeCtl::new();
		let err = install_start(&sc2, &start_opts("app", "app-web-1"), true, false)
			.expect_err("dry-run install_start must refuse");
		assert!(matches!(err, ComposeError::Autostart(_)), "got {err:?}");

		let after = std::fs::read(&path).unwrap();
		assert_eq!(
			before, after,
			"a refused dry-run must leave the file untouched"
		);
		assert!(
			sc2.systemctl_log().is_empty(),
			"dry-run refusal must not reach systemctl"
		);

		// Now install start for real, then dry-run service mode.
		let sc3 = FakeCtl::new();
		install_start(&sc3, &start_opts("app2", "app2-web-1"), false, false).unwrap();
		let path2 = root.join("systemd/user/podup-app2.service");
		let before2 = std::fs::read(&path2).unwrap();

		let mut dry_service = opts(root, "app2", true, false);
		dry_service.dry_run = true;
		let sc4 = FakeCtl::new();
		let err =
			install(&sc4, &dry_service).expect_err("dry-run install must refuse over a start unit");
		assert!(matches!(err, ComposeError::Autostart(_)), "got {err:?}");

		let after2 = std::fs::read(&path2).unwrap();
		assert_eq!(
			before2, after2,
			"dry-run refusal must leave the file untouched"
		);
		assert!(sc4.systemctl_log().is_empty());
	});
}

/// Every service-mode line a release has written must classify as service, or
/// the documented same-mode reinstall would be refused: the current renderer
/// with profiles, env files and a quoted path holding a space and a `%`, the
/// plain `up -d` of 1.9.3 to 5.10.5, and the `up -d --build` of 1.9.0 to 1.9.2.
#[test]
fn every_released_service_line_classifies_as_service() {
	let mut o = opts(std::path::Path::new("/srv/my app %h"), "up", false, false);
	o.unit.profiles = vec!["up".to_string()];
	o.unit.env_files = vec!["/srv/my app %h/.env".to_string()];
	let rendered = render_service_unit(&o.unit);
	let line = rendered
		.lines()
		.find_map(|l| l.strip_prefix("ExecStart="))
		.unwrap();
	assert_eq!(
		classify_exec_start(line),
		DetectedSingleUnitMode::Service,
		"{line}"
	);
	for legacy in [
		"/usr/local/bin/podup -f /srv/app/docker-compose.yml -p app up -d",
		"/usr/local/bin/podup -f /srv/app/docker-compose.yml -p app up -d --build",
	] {
		assert_eq!(
			classify_exec_start(legacy),
			DetectedSingleUnitMode::Service,
			"{legacy}"
		);
	}
}

/// A unit file with no `ExecStart=` is refused too, not treated as absent.
#[test]
fn a_unit_without_exec_start_is_refused() {
	with_env(|root| {
		let dir = root.join("systemd/user");
		std::fs::create_dir_all(&dir).unwrap();
		let path = dir.join("podup-app.service");
		std::fs::write(&path, "").unwrap();
		let sc = FakeCtl::new();
		install(&sc, &opts(root, "app", false, false)).expect_err("an empty unit must be refused");
		assert_eq!(std::fs::read(&path).unwrap(), b"");
	});
}
