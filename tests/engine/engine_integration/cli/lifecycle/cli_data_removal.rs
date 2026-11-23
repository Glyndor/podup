//! CLI binary integration tests for the commands that delete data, checked
//! against what Podman holds afterwards: `down` keeps a named volume and
//! `down -v` removes it, `up -V` replaces an anonymous volume, `up
//! --remove-orphans` removes the containers the file no longer declares, and
//! `rm -s` stops and removes a container even while it is paused.
use super::*;
use std::fs;
use std::process::Command;

const SLEEP: &str = "image: alpine:latest\n    command: [\"sleep\", \"infinity\"]";

/// Removes the volumes it was handed when it goes out of scope, so a failed
/// assertion cannot leave one behind in the store.
struct Volumes(std::sync::Mutex<Vec<String>>);

impl Volumes {
	fn new() -> Self {
		Self(std::sync::Mutex::new(Vec::new()))
	}

	fn track(&self, name: &str) {
		self.0.lock().unwrap().push(name.to_string());
	}
}

impl Drop for Volumes {
	fn drop(&mut self) {
		for name in self.0.lock().unwrap().iter() {
			let _ = Command::new("podman")
				.args(["volume", "rm", "-f", name])
				.output();
		}
	}
}

fn volume_exists(name: &str) -> bool {
	Command::new("podman")
		.args(["volume", "exists", name])
		.status()
		.expect("run podman volume exists")
		.success()
}

/// The names of the volumes mounted into the container, in mount order.
fn mounted_volumes(container: &str) -> Vec<String> {
	let out = Command::new("podman")
		.args([
			"inspect",
			container,
			"--format",
			"{{range .Mounts}}{{.Name}}\n{{end}}",
		])
		.output()
		.expect("run podman inspect");
	assert!(
		out.status.success(),
		"podman inspect {container}: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	String::from_utf8_lossy(&out.stdout)
		.lines()
		.map(str::trim)
		.filter(|l| !l.is_empty())
		.map(str::to_string)
		.collect()
}

fn container_exists(name: &str) -> bool {
	Command::new("podman")
		.args(["container", "exists", name])
		.status()
		.expect("run podman container exists")
		.success()
}

#[tokio::test]
async fn cli_down_keeps_a_named_volume_and_down_volumes_removes_it() {
	if super::podman().await.is_none() {
		return;
	}
	let guard = super::DownGuard::new(
		"vol",
		&format!("services:\n  web:\n    {SLEEP}\n    volumes:\n      - data:/data\nvolumes:\n  data: {{}}\n"),
	);
	let (c, proj) = (guard.compose_path(), guard.name());
	let volume = format!("{proj}_data");
	let volumes = Volumes::new();
	volumes.track(&volume);

	run_ok(&["-f", c, "-p", proj, "up", "-d"]);
	assert!(volume_exists(&volume), "up must create the named volume");

	run_ok(&["-f", c, "-p", proj, "down"]);
	assert!(
		volume_exists(&volume),
		"down without -v must keep the named volume and its data"
	);
	assert_eq!(ps_all_count(c, proj), 0, "down removes the container");

	run_ok(&["-f", c, "-p", proj, "down", "-v"]);
	assert!(
		!volume_exists(&volume),
		"down -v must remove the named volume"
	);
}

#[tokio::test]
async fn cli_down_volumes_leaves_an_external_volume_in_the_store() {
	if super::podman().await.is_none() {
		return;
	}
	// `proj` is the same function `DownGuard::new` names its project with, so the
	// external volume can carry the project's per-run prefix before the guard exists.
	let external = format!("{}-external", super::proj("extvol"));
	let volumes = Volumes::new();
	volumes.track(&external);
	let guard = super::DownGuard::new(
		"extvol",
		&format!(
			"services:\n  web:\n    {SLEEP}\n    volumes:\n      - shared:/data\nvolumes:\n  shared:\n    external: true\n    name: {external}\n"
		),
	);
	let (c, proj) = (guard.compose_path(), guard.name());
	let made = Command::new("podman")
		.args(["volume", "create", &external])
		.output()
		.unwrap();
	assert!(made.status.success(), "{made:?}");

	run_ok(&["-f", c, "-p", proj, "up", "-d"]);
	run_ok(&["-f", c, "-p", proj, "down", "-v"]);
	assert!(
		volume_exists(&external),
		"down -v must not remove a volume the project did not create"
	);
}

#[tokio::test]
async fn cli_up_renew_anon_volumes_replaces_the_anonymous_volume() {
	if super::podman().await.is_none() {
		return;
	}
	let guard = super::DownGuard::new(
		"renew",
		&format!("services:\n  web:\n    {SLEEP}\n    volumes:\n      - /scratch\n"),
	);
	let (c, proj) = (guard.compose_path(), guard.name());
	let container = format!("{proj}-web-1");
	let volumes = Volumes::new();

	run_ok(&["-f", c, "-p", proj, "up", "-d"]);
	let first = mounted_volumes(&container);
	assert_eq!(first.len(), 1, "one anonymous volume expected: {first:?}");
	first.iter().for_each(|v| volumes.track(v));

	// Control: a plain forced recreation keeps the volume the old container had.
	run_ok(&["-f", c, "-p", proj, "up", "-d", "--force-recreate"]);
	let second = mounted_volumes(&container);
	assert_eq!(
		second, first,
		"without -V the recreated container inherits its anonymous volume"
	);

	// With -V the volume is replaced and the old one is gone from the store.
	run_ok(&["-f", c, "-p", proj, "up", "-d", "--force-recreate", "-V"]);
	let third = mounted_volumes(&container);
	assert_eq!(third.len(), 1, "{third:?}");
	third.iter().for_each(|v| volumes.track(v));
	assert_ne!(third, first, "-V must give the container a fresh volume");
	assert!(
		!volume_exists(&first[0]),
		"-V must remove the anonymous volume it replaced"
	);
}

#[tokio::test]
async fn cli_up_remove_orphans_removes_undeclared_containers_and_only_with_the_flag() {
	if super::podman().await.is_none() {
		return;
	}
	// The guard owns the file that declares both services, so its teardown
	// reaches the orphan as well, even when an assertion fails halfway.
	let guard = super::DownGuard::new(
		"uporph",
		&format!("services:\n  web:\n    {SLEEP}\n  extra:\n    {SLEEP}\n"),
	);
	let (two, proj) = (guard.compose_path(), guard.name());
	let one = guard.dir().join("one.yml");
	fs::write(&one, format!("services:\n  web:\n    {SLEEP}\n")).unwrap();
	let one = one.to_str().unwrap();
	let extra = format!("{proj}-extra-1");

	run_ok(&["-f", two, "-p", proj, "up", "-d"]);
	assert!(container_exists(&extra), "the second service must exist");

	// Without the flag the container the file no longer declares stays.
	run_ok(&["-f", one, "-p", proj, "up", "-d"]);
	assert!(
		container_exists(&extra),
		"up without --remove-orphans must keep the orphan"
	);

	run_ok(&["-f", one, "-p", proj, "up", "-d", "--remove-orphans"]);
	assert!(
		!container_exists(&extra),
		"up --remove-orphans must remove the orphan"
	);
	assert!(
		container_exists(&format!("{proj}-web-1")),
		"the declared service must survive"
	);
}

#[tokio::test]
async fn cli_rm_stop_removes_a_paused_container() {
	if super::podman().await.is_none() {
		return;
	}
	let guard = super::DownGuard::new("rmpaused", &format!("services:\n  web:\n    {SLEEP}\n"));
	let (c, proj) = (guard.compose_path(), guard.name());

	run_ok(&["-f", c, "-p", proj, "up", "-d"]);
	run_ok(&["-f", c, "-p", proj, "pause"]);
	let state = Command::new("podman")
		.args([
			"inspect",
			&format!("{proj}-web-1"),
			"--format",
			"{{.State.Status}}",
		])
		.output()
		.unwrap();
	assert_eq!(
		String::from_utf8_lossy(&state.stdout).trim(),
		"paused",
		"the container must be paused before the removal"
	);

	let rm = run(&["-f", c, "-p", proj, "rm", "-s", "web"]);
	assert!(
		rm.status.success(),
		"rm -s on a paused container failed: {}",
		String::from_utf8_lossy(&rm.stderr)
	);
	assert_eq!(
		ps_all_count(c, proj),
		0,
		"rm -s must remove the paused container"
	);
}

#[tokio::test]
async fn cli_rm_without_stop_or_force_keeps_a_running_container() {
	if super::podman().await.is_none() {
		return;
	}
	let guard = super::DownGuard::new("rmkeeps", &format!("services:\n  web:\n    {SLEEP}\n"));
	let (c, proj) = (guard.compose_path(), guard.name());

	run_ok(&["-f", c, "-p", proj, "up", "-d"]);
	// The exit status is not asserted: it depends on how the engine words the
	// refusal. What matters here is that nothing was removed.
	run(&["-f", c, "-p", proj, "rm", "web"]);
	assert_eq!(
		ps_all_count(c, proj),
		1,
		"rm without -s or -f must not remove a running container"
	);
}
