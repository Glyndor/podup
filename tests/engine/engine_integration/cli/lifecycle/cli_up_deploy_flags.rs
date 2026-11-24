//! CLI binary integration tests for the `up` flags that change what gets
//! deployed, checked against the containers Podman holds afterwards:
//! `--no-recreate`, `--always-recreate-deps`, `--no-deps` and `--quiet-build`.
use super::*;
use std::fs;
use std::process::Command;

const SLEEP: &str = "image: alpine:latest\n    command: [\"sleep\", \"infinity\"]";

/// The ID Podman gives a container, or an empty string when it is not there.
fn container_id(name: &str) -> String {
	let out = Command::new("podman")
		.args(["inspect", name, "--format", "{{.Id}}"])
		.output()
		.expect("run podman inspect");
	String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn container_env(name: &str) -> String {
	let out = Command::new("podman")
		.args([
			"inspect",
			name,
			"--format",
			"{{range .Config.Env}}{{.}} {{end}}",
		])
		.output()
		.expect("run podman inspect");
	String::from_utf8_lossy(&out.stdout).to_string()
}

#[tokio::test]
async fn cli_up_no_recreate_keeps_a_container_whose_configuration_changed() {
	if super::podman().await.is_none() {
		return;
	}
	let guard = super::DownGuard::new(
		"norecreate",
		&format!("services:\n  web:\n    {SLEEP}\n    environment:\n      V: one\n"),
	);
	let (one, proj) = (guard.compose_path(), guard.name());
	let two = guard.dir().join("two.yml");
	fs::write(
		&two,
		format!("services:\n  web:\n    {SLEEP}\n    environment:\n      V: two\n"),
	)
	.unwrap();
	let two = two.to_str().unwrap();
	let web = format!("{proj}-web-1");

	run_ok(&["-f", one, "-p", proj, "up", "-d"]);
	let first = container_id(&web);
	assert!(!first.is_empty(), "web must exist after up");

	run_ok(&["-f", two, "-p", proj, "up", "-d", "--no-recreate"]);
	assert_eq!(
		container_id(&web),
		first,
		"--no-recreate must leave the existing container in place"
	);
	assert!(
		container_env(&web).contains("V=one"),
		"and with the configuration it was created with: {}",
		container_env(&web)
	);

	// The same change without the flag replaces the container.
	run_ok(&["-f", two, "-p", proj, "up", "-d"]);
	assert_ne!(
		container_id(&web),
		first,
		"without --no-recreate the changed configuration is deployed"
	);
	assert!(
		container_env(&web).contains("V=two"),
		"{}",
		container_env(&web)
	);
}

#[tokio::test]
async fn cli_up_always_recreate_deps_replaces_the_dependencies_and_not_the_named_service() {
	if super::podman().await.is_none() {
		return;
	}
	let guard = super::DownGuard::new(
		"recreatedeps",
		&format!("services:\n  web:\n    {SLEEP}\n    depends_on: [db]\n  db:\n    {SLEEP}\n"),
	);
	let (c, proj) = (guard.compose_path(), guard.name());
	let (web, db) = (format!("{proj}-web-1"), format!("{proj}-db-1"));

	run_ok(&["-f", c, "-p", proj, "up", "-d"]);
	let (web_id, db_id) = (container_id(&web), container_id(&db));
	assert!(!web_id.is_empty() && !db_id.is_empty());

	// Control: naming the service alone leaves an unchanged dependency alone.
	run_ok(&["-f", c, "-p", proj, "up", "-d", "web"]);
	assert_eq!(container_id(&db), db_id, "an unchanged dependency is kept");

	run_ok(&[
		"-f",
		c,
		"-p",
		proj,
		"up",
		"-d",
		"--always-recreate-deps",
		"web",
	]);
	assert_ne!(
		container_id(&db),
		db_id,
		"--always-recreate-deps replaces the dependency"
	);
	assert_eq!(
		container_id(&web),
		web_id,
		"the service named on the command line is not forced"
	);
}

#[tokio::test]
async fn cli_up_no_deps_leaves_the_dependency_uncreated() {
	if super::podman().await.is_none() {
		return;
	}
	let guard = super::DownGuard::new(
		"nodeps",
		&format!("services:\n  web:\n    {SLEEP}\n    depends_on: [db]\n  db:\n    {SLEEP}\n"),
	);
	let (c, proj) = (guard.compose_path(), guard.name());
	let (web, db) = (format!("{proj}-web-1"), format!("{proj}-db-1"));

	run_ok(&["-f", c, "-p", proj, "up", "-d", "--no-deps", "web"]);
	assert!(!container_id(&web).is_empty(), "web is created");
	assert!(
		container_id(&db).is_empty(),
		"--no-deps must not create the dependency"
	);

	// Without the flag the same command brings the dependency up too.
	run_ok(&["-f", c, "-p", proj, "up", "-d", "web"]);
	assert!(
		!container_id(&db).is_empty(),
		"without --no-deps the dependency is created"
	);
}

#[tokio::test]
async fn cli_up_quiet_build_hides_the_build_output() {
	if super::podman().await.is_none() {
		return;
	}
	let _build = super::BUILD_IMAGES.lock().await;
	let pid = std::process::id();
	let image = format!("podup-test-quietbuild-{pid}:latest");
	let _image = super::TestImage::new(image.clone());
	let guard = super::DownGuard::new(
		"quietbuild",
		&format!(
			"services:\n  web:\n    image: {image}\n    command: [\"sleep\", \"infinity\"]\n    build:\n      context: .\n      dockerfile_inline: |\n        FROM alpine:latest\n        RUN echo quiet-build-marker-{pid}\n"
		),
	);
	let (c, proj) = (guard.compose_path(), guard.name());

	let loud = run_ok(&["-f", c, "-p", proj, "up", "-d", "--build"]);
	let loud = String::from_utf8_lossy(&loud.stderr).to_string();
	assert!(
		loud.contains(&format!("quiet-build-marker-{pid}")),
		"without the flag the build steps are shown: {loud}"
	);

	let quiet = run_ok(&["-f", c, "-p", proj, "up", "-d", "--build", "--quiet-build"]);
	let quiet = String::from_utf8_lossy(&quiet.stderr).to_string();
	assert!(
		!quiet.contains(&format!("quiet-build-marker-{pid}")),
		"--quiet-build must hide the build steps: {quiet}"
	);
}
