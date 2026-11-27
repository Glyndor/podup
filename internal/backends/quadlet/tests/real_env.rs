//! The environment of the programs the tests that need a real Podman or
//! systemd run.
//!
//! The unit suite is one process, and other tests change `XDG_RUNTIME_DIR`,
//! `XDG_CONFIG_HOME`, `HOME` and `PATH` of that process while they run: the
//! autostart tests point them at temporary directories and fake programs. A
//! program started in that window inherits them, and `systemctl --user` then
//! cannot reach the user manager. So a program run here is given an
//! environment built from the user id and from the user manager, never from
//! what this process holds at that moment.

use std::ffi::OsStr;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// Where programs are looked for, whatever `PATH` says now.
pub(in crate::backends::quadlet) const PATH: &str =
	"/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// The variables that say where the user's files are, taken from the user
/// manager so that a `podman` run here sees the storage the units see.
const FROM_MANAGER: [&str; 3] = ["HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME"];

/// The user this process runs as, read as the owner of a directory it has just
/// made: there is no `/proc` on macOS, and the crate has no `geteuid` here.
fn uid() -> u32 {
	let made = tempfile::tempdir().expect("a temporary directory");
	std::fs::metadata(made.path())
		.expect("the directory just made")
		.uid()
}

/// `program` with what it takes to reach the user manager, and nothing else.
fn bare(program: impl AsRef<OsStr>) -> Command {
	let runtime = format!("/run/user/{}", uid());
	let mut command = Command::new(program);
	command
		.env_clear()
		.env("PATH", PATH)
		.env(
			"DBUS_SESSION_BUS_ADDRESS",
			format!("unix:path={runtime}/bus"),
		)
		.env("XDG_RUNTIME_DIR", runtime);
	command
}

/// What the user manager has for `name`, if it answers and has it.
fn manager_value(name: &str) -> Option<String> {
	let shown = bare("systemctl")
		.args(["--user", "show-environment"])
		.output()
		.ok()?;
	String::from_utf8_lossy(&shown.stdout)
		.lines()
		.find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
		.map(str::to_string)
}

/// The home directory `/etc/passwd` gives this user, for a machine with no
/// user manager to ask.
fn passwd_home() -> Option<String> {
	let uid = uid().to_string();
	std::fs::read_to_string("/etc/passwd")
		.ok()?
		.lines()
		.map(|line| line.split(':').collect::<Vec<_>>())
		.find(|fields| fields.get(2) == Some(&uid.as_str()))
		.and_then(|fields| fields.get(5).map(|home| home.to_string()))
}

/// [`FROM_MANAGER`] as this machine has them, asked for once.
fn user_files() -> &'static [(&'static str, String)] {
	static FOUND: OnceLock<Vec<(&str, String)>> = OnceLock::new();
	FOUND.get_or_init(|| {
		FROM_MANAGER
			.into_iter()
			.filter_map(|name| {
				let value =
					manager_value(name).or_else(|| (name == "HOME").then(passwd_home).flatten())?;
				Some((name, value))
			})
			.collect()
	})
}

/// `program`, to be run with an environment of its own.
pub(in crate::backends::quadlet) fn command(program: impl AsRef<OsStr>) -> Command {
	let mut command = bare(program);
	command.envs(user_files().iter().map(|(name, value)| (name, value)));
	command
}

/// The directory the user manager links units into.
pub(in crate::backends::quadlet) fn manager_unit_dir() -> Option<PathBuf> {
	let value = |wanted: &str| {
		user_files()
			.iter()
			.find(|(name, _)| *name == wanted)
			.map(|(_, value)| PathBuf::from(value))
	};
	value("XDG_CONFIG_HOME")
		.or_else(|| value("HOME").map(|home| home.join(".config")))
		.map(|config| config.join("systemd").join("user"))
}

/// Another test's environment does not reach a program run from here.
#[test]
fn a_program_run_here_does_not_inherit_the_environment_a_test_changed() {
	let elsewhere = tempfile::tempdir().unwrap();
	let other = elsewhere.path().to_str().unwrap();
	temp_env::with_vars(
		[
			("XDG_RUNTIME_DIR", Some(other)),
			("DBUS_SESSION_BUS_ADDRESS", Some(other)),
			("XDG_CONFIG_HOME", Some(other)),
			("XDG_DATA_HOME", Some(other)),
			("HOME", Some(other)),
			("PATH", Some(other)),
			("USER", Some("tester")),
		],
		|| {
			let out = command("sh").args(["-c", "env"]).output().unwrap();
			let seen = String::from_utf8_lossy(&out.stdout);
			assert!(out.status.success());
			assert!(!seen.contains(other), "{seen}");
			assert!(!seen.contains("USER=tester"), "{seen}");
			assert_ne!(
				manager_unit_dir(),
				Some(elsewhere.path().join("systemd/user"))
			);
		},
	);
}
