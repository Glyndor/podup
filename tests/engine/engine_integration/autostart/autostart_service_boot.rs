//! The service-mode autostart unit neither builds nor pulls at boot (#1951).
//!
//! Every test runs the command the unit actually carries, not a hand-typed
//! copy: it takes the `ExecStart=` line from `autostart install --dry-run`,
//! which prints the unit and writes nothing, and runs its arguments through
//! the test binary. Removing a flag from `render_service_unit` therefore
//! changes what these tests execute.
//!
//! Each test gathers what it asserts on, tears its fixture down, and only
//! then asserts, so a failing assertion does not leave containers or images
//! behind.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

use super::{bin, podman_socket_url, proj, run};

/// The arguments of the rendered `ExecStart=` line, without the executable.
/// Splitting on whitespace is safe here: tempdir paths and `t<pid>-` project
/// names contain no spaces.
fn rendered_exec_args(compose: &Path, project: &str) -> Vec<String> {
	let out = Command::new(bin())
		.arg("-f")
		.arg(compose)
		.args(["-p", project, "autostart", "install", "--dry-run"])
		.output()
		.expect("run podup autostart install --dry-run");
	let stdout = String::from_utf8_lossy(&out.stdout);
	assert!(
		out.status.success(),
		"`autostart install --dry-run` failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let line = stdout
		.lines()
		.find_map(|l| l.strip_prefix("ExecStart="))
		.unwrap_or_else(|| panic!("no ExecStart= line in dry-run output:\n{stdout}"));
	line.split_whitespace()
		.skip(1)
		.map(str::to_string)
		.collect()
}

/// Run what the boot unit would run.
fn run_boot(compose: &Path, project: &str) -> Output {
	Command::new(bin())
		.args(rendered_exec_args(compose, project))
		.output()
		.expect("run podup")
}

fn image_exists(socket: &str, tag: &str) -> bool {
	Command::new("podman")
		.args(["--url", socket, "image", "exists", tag])
		.status()
		.is_ok_and(|s| s.success())
}

fn rmi(socket: &str, tag: &str) {
	let _ = Command::new("podman")
		.args(["--url", socket, "rmi", "-f", tag])
		.output();
}

fn combined(out: &Output) -> String {
	format!(
		"{}{}",
		String::from_utf8_lossy(&out.stdout),
		String::from_utf8_lossy(&out.stderr)
	)
}

fn down(compose: &Path, project: &str) {
	run(&["-f", compose.to_str().unwrap(), "-p", project, "down", "-v"]);
}

/// A `build:` service whose image is not on the host. The boot must fail
/// and the image must still be absent afterwards: the absence is what names
/// the defect, since the old unit built it and exited 0.
#[tokio::test]
async fn boot_does_not_build_a_missing_image() {
	let Some(socket) = podman_socket_url() else {
		return;
	};
	if super::podman().await.is_none() {
		return;
	}
	let dir = TempDir::new().unwrap();
	let project = proj("bootnobuild");
	let tag = format!("localhost/{project}-app:latest");
	// `FROM scratch` needs no base image, so an attempted build succeeds
	// offline and leaves the tag behind. A base that had to be fetched could
	// fail the build on a host without the network and hide the defect.
	fs::write(
		dir.path().join("Dockerfile"),
		"FROM scratch\nLABEL podup.test=boot\n",
	)
	.unwrap();
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		format!("services:\n  app:\n    build: .\n    image: {tag}\n    command: [\"sleep\", \"infinity\"]\n"),
	)
	.unwrap();
	assert!(
		!image_exists(&socket, &tag),
		"fixture image {tag} already present"
	);

	let out = run_boot(&compose, &project);
	let built = image_exists(&socket, &tag);

	down(&compose, &project);
	if built {
		rmi(&socket, &tag);
	}

	assert!(
		!built,
		"the boot unit built {tag}; output was:\n{}",
		combined(&out)
	);
	assert!(
		!out.status.success(),
		"a boot with a missing image must fail the unit; output was:\n{}",
		combined(&out)
	);
}

/// An `image:` service whose image is not on the host, on a registry nothing
/// listens on (port 1), so the test needs no network. `no such image` is the
/// create-time 404; a pull attempt fails before any container is created and
/// reports a connection error instead, so the string only appears when no
/// pull was tried.
#[tokio::test]
async fn boot_does_not_pull_a_missing_image() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = TempDir::new().unwrap();
	let project = proj("bootnopull");
	let image = format!("localhost:1/{project}:latest");
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		format!("services:\n  web:\n    image: {image}\n    command: [\"sleep\", \"infinity\"]\n"),
	)
	.unwrap();

	let out = run_boot(&compose, &project);
	down(&compose, &project);

	let text = combined(&out);
	assert!(
		!out.status.success(),
		"a boot with a missing image must fail the unit; output was:\n{text}"
	);
	assert!(
		text.contains("no such image"),
		"the boot unit tried to pull {image}; output was:\n{text}"
	);
}

/// What a deploy left on disk, the boot starts. Without this, a unit that
/// never starts anything would satisfy both tests above.
#[tokio::test]
async fn boot_starts_what_the_last_deploy_left() {
	let Some(socket) = podman_socket_url() else {
		return;
	};
	if super::podman().await.is_none() {
		return;
	}
	let dir = TempDir::new().unwrap();
	let project = proj("bootstarts");
	let built_tag = format!("localhost/{project}-app:latest");
	fs::write(dir.path().join("Dockerfile"), "FROM alpine:latest\n").unwrap();
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		format!(
			"services:\n  app:\n    build: .\n    image: {built_tag}\n    command: [\"sleep\", \"infinity\"]\n  \
			 web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n"
		),
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	let deploy = run(&["-f", c, "-p", &project, "up", "-d"]);
	let stop = run(&["-f", c, "-p", &project, "stop"]);
	let out = run_boot(&compose, &project);
	let states: Vec<String> = ["app", "web"]
		.iter()
		.map(|svc| {
			let name = format!("{project}-{svc}-1");
			Command::new("podman")
				.args([
					"--url",
					&socket,
					"inspect",
					"--format",
					"{{.State.Status}}",
					&name,
				])
				.output()
				.map(|o| format!("{name}={}", String::from_utf8_lossy(&o.stdout).trim()))
				.unwrap_or_else(|e| format!("{name}: {e}"))
		})
		.collect();

	down(&compose, &project);
	// Only the image this test built; `alpine:latest` is shared.
	rmi(&socket, &built_tag);

	assert!(
		deploy.status.success(),
		"deploy failed:\n{}",
		combined(&deploy)
	);
	assert!(stop.status.success(), "stop failed:\n{}", combined(&stop));
	assert!(
		out.status.success(),
		"the boot unit failed although the deploy left every image on disk:\n{}",
		combined(&out)
	);
	assert_eq!(
		states,
		[
			format!("{project}-app-1=running"),
			format!("{project}-web-1=running")
		],
		"both containers must be running after the boot"
	);
}
