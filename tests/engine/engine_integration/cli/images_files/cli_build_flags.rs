//! CLI binary integration tests for the `build` flags, checked against the
//! images Podman holds afterwards: `-q`, `--build-arg`, `--pull`, `--push` and
//! `--with-dependencies`. Nothing here reaches a network: the base image is
//! `alpine:latest` or a local tag, and the registry a push is aimed at refuses
//! the connection.
use super::*;
use std::fs;
use std::process::Command;

fn image_exists(tag: &str) -> bool {
	Command::new("podman")
		.args(["image", "exists", tag])
		.status()
		.expect("run podman image exists")
		.success()
}

/// What the image holds in `/arg`, read by running a throwaway container.
fn baked_arg(tag: &str) -> String {
	let out = Command::new("podman")
		.args(["run", "--rm", "--pull=never", tag, "cat", "/arg"])
		.output()
		.expect("run podman run");
	assert!(
		out.status.success(),
		"podman run {tag}: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn compose(dir: &std::path::Path, body: &str) -> String {
	let path = dir.join("compose.yaml");
	fs::write(&path, body).unwrap();
	path.to_str().unwrap().to_string()
}

fn build_project(tag: &str) -> String {
	// Through `proj`, so the leak scan knows the run that made the resources.
	super::proj(tag)
}

#[tokio::test]
async fn cli_build_quiet_hides_the_build_steps() {
	if super::podman().await.is_none() {
		return;
	}
	let _build = super::BUILD_IMAGES.lock().await;
	let pid = std::process::id();
	let image = format!("podup-test-bquiet-{pid}:latest");
	let _image = super::TestImage::new(image.clone());
	let dir = tempfile::tempdir().unwrap();
	let c = compose(
		dir.path(),
		&format!(
			"services:\n  app:\n    image: {image}\n    build:\n      context: .\n      dockerfile_inline: |\n        FROM alpine:latest\n        RUN echo build-quiet-marker-{pid}\n"
		),
	);
	let proj = build_project("bquiet");

	let loud = run_ok(&["-f", &c, "-p", &proj, "build", "--no-cache"]);
	assert!(
		String::from_utf8_lossy(&loud.stderr).contains(&format!("build-quiet-marker-{pid}")),
		"without -q the steps are shown: {}",
		String::from_utf8_lossy(&loud.stderr)
	);

	for flag in ["-q", "--quiet"] {
		let quiet = run_ok(&["-f", &c, "-p", &proj, "build", "--no-cache", flag]);
		let stderr = String::from_utf8_lossy(&quiet.stderr);
		assert!(
			!stderr.contains(&format!("build-quiet-marker-{pid}")),
			"{flag} must hide the build steps: {stderr}"
		);
		assert!(image_exists(&image), "{flag} still builds the image");
	}
}

#[tokio::test]
async fn cli_build_arg_reaches_the_build() {
	if super::podman().await.is_none() {
		return;
	}
	let _build = super::BUILD_IMAGES.lock().await;
	let pid = std::process::id();
	let image = format!("podup-test-barg-{pid}:latest");
	let _image = super::TestImage::new(image.clone());
	let dir = tempfile::tempdir().unwrap();
	let c = compose(
		dir.path(),
		&format!(
			"services:\n  app:\n    image: {image}\n    build:\n      context: .\n      dockerfile_inline: |\n        FROM alpine:latest\n        ARG PODUP_TEST_ARG=unset\n        RUN echo \"$$PODUP_TEST_ARG\" > /arg\n"
		),
	);
	let proj = build_project("barg");

	run_ok(&["-f", &c, "-p", &proj, "build"]);
	assert_eq!(
		baked_arg(&image),
		"unset",
		"without the flag the default of the ARG is used"
	);

	run_ok(&[
		"-f",
		&c,
		"-p",
		&proj,
		"build",
		"--build-arg",
		&format!("PODUP_TEST_ARG=from-cli-{pid}"),
	]);
	assert_eq!(
		baked_arg(&image),
		format!("from-cli-{pid}"),
		"--build-arg must reach the build"
	);
}

#[tokio::test]
async fn cli_build_pull_asks_the_registry_for_the_base_image() {
	if super::podman().await.is_none() {
		return;
	}
	let _build = super::BUILD_IMAGES.lock().await;
	let pid = std::process::id();
	// A base image that exists only here: it builds from the local store, and
	// asking a registry for it can only fail.
	let base = format!("localhost:1/podup-test-base-{pid}:1");
	let tagged = Command::new("podman")
		.args(["tag", "alpine:latest", &base])
		.status()
		.unwrap();
	assert!(tagged.success(), "tagging the local base image failed");
	let _base = super::TestImage::new(base.clone());
	let image = format!("podup-test-bpull-{pid}:latest");
	let _image = super::TestImage::new(image.clone());
	let dir = tempfile::tempdir().unwrap();
	let c = compose(
		dir.path(),
		&format!(
			"services:\n  app:\n    image: {image}\n    build:\n      context: .\n      dockerfile_inline: |\n        FROM {base}\n        RUN true\n"
		),
	);
	let proj = build_project("bpull");

	run_ok(&["-f", &c, "-p", &proj, "build"]);

	let pulled = run(&["-f", &c, "-p", &proj, "build", "--pull"]);
	let stderr = String::from_utf8_lossy(&pulled.stderr);
	assert!(
		!pulled.status.success(),
		"--pull must go to the registry for the base image: {stderr}"
	);
	assert!(
		stderr.contains("localhost:1") || stderr.contains("podup-test-base"),
		"the failure names the image that could not be pulled: {stderr}"
	);
}

#[tokio::test]
async fn cli_build_push_pushes_after_the_build_and_without_it_nothing_is_pushed() {
	if super::podman().await.is_none() {
		return;
	}
	let _build = super::BUILD_IMAGES.lock().await;
	let pid = std::process::id();
	// Port 1 refuses connections, so a push can only fail, and the build before
	// it has already put the image in the local store.
	let image = format!("localhost:1/podup-test-bpush-{pid}:1");
	let _image = super::TestImage::new(image.clone());
	let dir = tempfile::tempdir().unwrap();
	let c = compose(
		dir.path(),
		&format!(
			"services:\n  app:\n    image: {image}\n    build:\n      context: .\n      dockerfile_inline: |\n        FROM alpine:latest\n        RUN true\n"
		),
	);
	let proj = build_project("bpush");

	run_ok(&["-f", &c, "-p", &proj, "build"]);

	let pushed = run(&["-f", &c, "-p", &proj, "build", "--push"]);
	let stderr = String::from_utf8_lossy(&pushed.stderr);
	assert!(
		!pushed.status.success(),
		"--push to a registry that refuses the connection must fail: {stderr}"
	);
	assert!(
		stderr.contains("localhost:1"),
		"the failure names the registry: {stderr}"
	);
	assert!(
		image_exists(&image),
		"the build before the push is kept in the local store"
	);
}

#[tokio::test]
async fn cli_build_with_dependencies_builds_what_the_service_depends_on() {
	if super::podman().await.is_none() {
		return;
	}
	let _build = super::BUILD_IMAGES.lock().await;
	let pid = std::process::id();
	let (web, db, other) = (
		format!("podup-test-bwd-web-{pid}:latest"),
		format!("podup-test-bwd-db-{pid}:latest"),
		format!("podup-test-bwd-other-{pid}:latest"),
	);
	let _images = [&web, &db, &other].map(|tag| super::TestImage::new(tag.clone()));
	let dir = tempfile::tempdir().unwrap();
	let service = |name: &str, tag: &str, depends: &str| {
		format!(
			"  {name}:\n    image: {tag}\n{depends}    build:\n      context: .\n      dockerfile_inline: |\n        FROM alpine:latest\n        RUN echo {name}\n"
		)
	};
	let c = compose(
		dir.path(),
		&format!(
			"services:\n{}{}{}",
			service("web", &web, "    depends_on: [db]\n"),
			service("db", &db, ""),
			service("other", &other, ""),
		),
	);
	let proj = build_project("bwd");

	run_ok(&["-f", &c, "-p", &proj, "build", "web"]);
	assert!(image_exists(&web), "the named service is built");
	assert!(
		!image_exists(&db),
		"without the flag its dependency is not built"
	);

	run_ok(&["-f", &c, "-p", &proj, "build", "--with-dependencies", "web"]);
	assert!(
		image_exists(&db),
		"--with-dependencies builds the dependency"
	);
	assert!(
		!image_exists(&other),
		"and not a service the named one does not depend on"
	);
}
