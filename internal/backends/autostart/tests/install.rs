use super::*;
use std::os::unix::fs::PermissionsExt;

/// The unit directory and the units inside it are private to their owner.
///
/// `create_dir_all` and `fs::write` take their permissions from the process
/// umask, so a permissive umask leaves the directory and the units in it
/// writable by any local user, and a unit another user can rewrite is a
/// command that user can run at boot.
///
/// The modes are set explicitly, so they hold whatever the umask is: a umask
/// can only take bits away, and `0700`/`0600` survive every umask that
/// leaves the files usable.
#[test]
fn autostart_creates_a_private_unit_directory_and_private_units() {
	with_env(|root| {
		let sc = FakeCtl::new();
		install(&sc, &opts_with_interval(root, "app", false, true, "daily")).unwrap();
		let dir = root.join("systemd/user");
		let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o7777;
		assert_eq!(
			dir_mode, 0o700,
			"the unit directory must be private: {:04o}",
			dir_mode
		);
		for name in [
			"podup-app.service",
			"podup-app.update.service",
			"podup-app.update.timer",
		] {
			let path = dir.join(name);
			let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777;
			assert_eq!(mode, 0o600, "{name} must be private: {:04o}", mode);
		}
	});
}

/// A unit written earlier under a permissive umask does not stay writable
/// after a re-install: the mode is applied again, not only at creation.
#[test]
fn autostart_tightens_the_mode_of_a_unit_that_already_existed() {
	with_env(|root| {
		let dir = root.join("systemd/user");
		std::fs::create_dir_all(&dir).unwrap();
		let path = dir.join("podup-app.service");
		std::fs::write(
			&path,
			"\
[Unit]
Description=podup app

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/usr/local/bin/podup -f /srv/app/docker-compose.yml -p app up -d --no-build --pull never
ExecStop=/usr/local/bin/podup -f /srv/app/docker-compose.yml -p app stop
",
		)
		.unwrap();
		std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();

		let sc = FakeCtl::new();
		install(&sc, &opts(root, "app", false, true)).unwrap();

		let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777;
		assert_eq!(
			mode, 0o600,
			"the re-installed unit must be private: {:04o}",
			mode
		);
	});
}

#[test]
fn install_writes_unit_and_enables() {
	with_env(|root| {
		let sc = FakeCtl::new();
		install(&sc, &opts(root, "app", false, false)).unwrap();
		let path = root.join("systemd/user/podup-app.service");
		assert!(path.is_file(), "unit file written");
		let body = std::fs::read_to_string(&path).unwrap();
		assert!(body.contains("Description=podup app"));
		let calls = sc.systemctl_log();
		assert_eq!(calls[0], vec!["daemon-reload"]);
		assert_eq!(calls[1], vec!["enable", "--now", "podup-app.service"]);
	});
}

#[test]
fn install_no_start_skips_enable() {
	with_env(|root| {
		let sc = FakeCtl::new();
		install(&sc, &opts(root, "app", false, true)).unwrap();
		let calls = sc.systemctl_log();
		assert_eq!(calls, vec![vec!["daemon-reload"]]);
	});
}

#[test]
fn dry_run_writes_nothing_and_runs_no_systemctl() {
	with_env(|root| {
		let sc = FakeCtl::new();
		install(&sc, &opts(root, "app", true, false)).unwrap();
		assert!(!root.join("systemd/user/podup-app.service").exists());
		assert!(sc.systemctl_log().is_empty());
	});
}

#[test]
fn install_refuses_on_quadlet_conflict() {
	with_env(|root| {
		let qdir = root.join("containers/systemd");
		std::fs::create_dir_all(&qdir).unwrap();
		std::fs::write(qdir.join("app-web.container"), b"[Container]\n").unwrap();
		let sc = FakeCtl::new();
		let err = install(&sc, &opts(root, "app", false, false)).unwrap_err();
		assert!(matches!(err, ComposeError::Autostart(_)));
		assert!(err.to_string().contains("quadlet"));
		// Nothing was installed.
		assert!(!root.join("systemd/user/podup-app.service").exists());
		assert!(sc.systemctl_log().is_empty());
	});
}

#[test]
fn install_surfaces_systemctl_failure() {
	with_env(|root| {
		let mut sc = FakeCtl::new();
		sc.fail = true;
		let err = install(&sc, &opts(root, "app", false, false)).unwrap_err();
		assert!(matches!(err, ComposeError::Autostart(_)));
	});
}

/// #1093: the unit's `TimeoutStopSec=` is derived from the project's slowest
/// service, so the roll-up must pick the maximum and tolerate the rest.
#[test]
fn max_stop_grace_picks_the_longest() {
	let file = crate::parse_str(
		"services:\n  a:\n    image: x\n    stop_grace_period: 10s\n  b:\n    image: x\n    stop_grace_period: 2m\n",
	)
	.unwrap();
	assert_eq!(max_stop_grace_secs(&file), Some(120));
}

/// No service setting one yields `None`, so the unit omits the key and systemd
/// keeps its own default rather than podup restating it.
#[test]
fn max_stop_grace_is_none_when_unset() {
	let file = crate::parse_str("services:\n  a:\n    image: x\n").unwrap();
	assert_eq!(max_stop_grace_secs(&file), None);
}

/// An unparseable duration is skipped rather than defaulted: the value is
/// validated elsewhere, and guessing a timeout from a malformed one would be
/// worse than not setting it. A valid sibling still counts.
#[test]
fn max_stop_grace_skips_an_unparseable_value() {
	let file = crate::parse_str(
		"services:\n  a:\n    image: x\n    stop_grace_period: \"nonsense\"\n  b:\n    image: x\n    stop_grace_period: 30s\n",
	)
	.unwrap();
	assert_eq!(max_stop_grace_secs(&file), Some(30));
}
