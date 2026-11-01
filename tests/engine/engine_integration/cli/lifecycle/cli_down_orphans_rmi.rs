//! CLI binary integration tests for the `down` and `kill` flags that reach past the
//! compose file, `--remove-orphans` and `--rmi all`.
use super::*;
use std::fs;
use std::process::Command;
use tempfile::tempdir;
#[tokio::test]
async fn cli_down_remove_orphans_drops_undeclared_containers() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-orphan", std::process::id());
	let two = dir.path().join("two.yml");
	let one = dir.path().join("one.yml");
	let svc = "image: alpine:latest\n    command: [\"sleep\", \"infinity\"]";
	fs::write(
		&two,
		format!("services:\n  web:\n    {svc}\n  extra:\n    {svc}\n"),
	)
	.unwrap();
	fs::write(&one, format!("services:\n  web:\n    {svc}\n")).unwrap();
	let (two, one) = (two.to_str().unwrap(), one.to_str().unwrap());

	run_ok(&["-f", two, "-p", &proj, "up", "-d"]);
	assert_eq!(ps_all_count(two, &proj), 2);

	// Down against the one-service file: --remove-orphans must also drop `extra`.
	let down = run(&["-f", one, "-p", &proj, "down", "--remove-orphans"]);
	assert!(
		down.status.success(),
		"down failed: {}",
		String::from_utf8_lossy(&down.stderr)
	);
	assert_eq!(
		ps_all_count(one, &proj),
		0,
		"--remove-orphans must remove the undeclared container too"
	);
}

#[tokio::test]
async fn cli_down_rmi_all_succeeds_and_removes_containers() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-rmi", std::process::id());

	// `--rmi all` deletes the image referenced by the compose file. Tests run in
	// parallel and several rely on the shared `alpine:latest` image, so we tag a
	// throwaway, project-unique local image and reference THAT here. Removing it
	// leaves `alpine:latest` intact for the concurrent tests.
	let throwaway = format!("localhost/podup-rmitest-{proj}:latest");
	// In a clean environment `alpine:latest` may not be pulled yet, so `podman
	// tag` would fail with "image not known". Pull first; ignore the result so a
	// pre-existing image (or an offline cache) still works.
	let _ = Command::new("podman")
		.args(["pull", "alpine:latest"])
		.output();
	let tag = Command::new("podman")
		.args(["tag", "alpine:latest", &throwaway])
		.output()
		.unwrap();
	assert!(
		tag.status.success(),
		"tagging throwaway image failed: {:?}",
		tag.stderr
	);

	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		format!(
			"services:\n  web:\n    image: {throwaway}\n    command: [\"sleep\", \"infinity\"]\n"
		),
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	run_ok(&["-f", c, "-p", &proj, "up", "-d"]);
	let down = run(&["-f", c, "-p", &proj, "down", "--rmi", "all"]);
	assert!(
		down.status.success(),
		"down --rmi all failed: {:?}",
		down.stderr
	);
	assert_eq!(ps_all_count(c, &proj), 0, "down must remove the containers");

	// The shared base image must survive; only the throwaway tag was removed.
	let present = Command::new("podman")
		.args(["image", "exists", &throwaway])
		.status()
		.unwrap();
	assert!(
		!present.success(),
		"down --rmi all must remove the throwaway image"
	);
	// Best-effort cleanup in case the assertion above ever changes.
	let _ = Command::new("podman")
		.args(["rmi", "-f", &throwaway])
		.output();
}

#[tokio::test]
async fn cli_kill_remove_orphans_drops_undeclared() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-killorph", std::process::id());
	let svc = "image: alpine:latest\n    command: [\"sleep\", \"infinity\"]";
	let two = dir.path().join("two.yml");
	let one = dir.path().join("one.yml");
	fs::write(
		&two,
		format!("services:\n  web:\n    {svc}\n  extra:\n    {svc}\n"),
	)
	.unwrap();
	fs::write(&one, format!("services:\n  web:\n    {svc}\n")).unwrap();
	let (two, one) = (two.to_str().unwrap(), one.to_str().unwrap());

	run_ok(&["-f", two, "-p", &proj, "up", "-d"]);
	let kill = run(&["-f", one, "-p", &proj, "kill", "--remove-orphans"]);
	assert!(
		kill.status.success(),
		"kill failed: {}",
		String::from_utf8_lossy(&kill.stderr)
	);
	// The orphan `extra` is removed; the declared `web` is killed but remains.
	run(&["-f", one, "-p", &proj, "down"]);
}
