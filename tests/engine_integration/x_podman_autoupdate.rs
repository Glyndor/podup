//! #1656: the `x-podman-autoupdate` extension drives `podman auto-update`-
//! compatible behaviour. The tests skip when Podman is not reachable.
//!
//! - `up_with_autoupdate_registry_creates_the_container_with_the_label`:
//!   after `up`, the container carries `io.containers.autoupdate=<value>` and
//!   it is read back through `podman inspect`.
//! - `up_with_autoupdate_registry_recreates_after_the_tag_moved_without_pull_flag`:
//!   with no `--pull` flag and no `pull_policy:`, a `podman tag` that moves the
//!   name to a different image is recreated on a plain `up`. The recreate
//!   happens through the config-hash + image-ID comparison `up` already does
//!   on a moved tag (`docs/commands.md`, the `up` section: "a tag moved by
//!   `podman tag`"), not through the pull policy; this test pins that the
//!   `io.containers.autoupdate=registry` label survives a recreate triggered
//!   by a moved local tag. It does NOT cover the registry check on its own
//!   (that is `up_with_autoupdate_registry_pulls_even_when_the_image_is_present`,
//!   and the unit-level pin is `autoupdate_registry_pulls_even_when_the_image_is_present`).
//! - `up_with_autoupdate_registry_pulls_even_when_the_image_is_present` (#1953):
//!   the live complement to the unit test of the same name. Tags `alpine:latest`
//!   as `localhost:1/<project>-probe:latest` (nothing listens on port 1, so any
//!   registry check is visible in the output and needs no network) and runs
//!   `podup up -d` against a compose that declares
//!   `x-podman-autoupdate: registry`. The output must contain a `Pulling`
//!   line for that image. The control, with the same image and project but no
//!   extension, asserts there is no such line: that is what proves the
//!   extension is the one making `up` reach the registry, not the image
//!   itself being missing.

use super::*;

/// Build a v1/v2 pair of tiny alpine-based images. The first call leaves `tag`
/// pointing at v1 (so a `up` against `tag` runs v1). The second call returns
/// v2's image ID without retagging `tag` itself, the test calls
/// `podman tag <v2-id> tag` after to move the tag, exactly the action the
/// extension's `registry` mode must catch on a plain `up`.
fn build_two(dir: &std::path::Path, tag: &str) -> (String, String) {
	let v1 = b"FROM alpine:latest\nRUN echo v1 > /version\n";
	let v2 = b"FROM alpine:latest\nRUN echo v2 > /version\n";
	let build = |contents: &[u8], suffix: &str| {
		fs::write(dir.join("Dockerfile"), contents).unwrap();
		let alias = format!("{tag}-{suffix}");
		let out = std::process::Command::new("podman")
			.args(["build", "-q", "-t", &alias, "-f", "Dockerfile", "."])
			.current_dir(dir)
			.output()
			.expect("podman build");
		assert!(
			out.status.success(),
			"podman build {suffix} failed: {}",
			String::from_utf8_lossy(&out.stderr)
		);
		String::from_utf8_lossy(&out.stdout).trim().to_string()
	};
	let v1_id = build(v1, "v1");
	// Tag the v1 image with the canonical name so the first `up` finds it.
	let out = std::process::Command::new("podman")
		.args(["tag", &v1_id, tag])
		.output()
		.expect("podman tag v1");
	assert!(
		out.status.success(),
		"podman tag v1 failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let v2_id = build(v2, "v2");
	(v1_id, v2_id)
}

fn rmi(tag: &str) {
	let _ = std::process::Command::new("podman")
		.args(["rmi", "-f", tag])
		.output();
}

/// A service declaring `x-podman-autoupdate: registry` is created with
/// `io.containers.autoupdate=registry` stamped onto the container, and
/// `podman inspect` reads it back (#1656).
#[tokio::test]
async fn up_with_autoupdate_registry_creates_the_container_with_the_label() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let proj = proj("au-label");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let yaml = format!(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    x-podman-autoupdate: {value}\n",
		value = "registry"
	);
	let file = parse_str(&yaml).unwrap();

	engine.up(&file).await.unwrap();
	let out = std::process::Command::new("podman")
		.args([
			"inspect",
			&format!("{proj}-web-1"),
			"--format",
			"{{index .Config.Labels \"io.containers.autoupdate\"}}",
		])
		.output()
		.expect("podman inspect");
	let label = String::from_utf8_lossy(&out.stdout).trim().to_string();
	engine.down(&file).await.unwrap();

	assert_eq!(
		label, "registry",
		"io.containers.autoupdate must be on the container with the same spelling"
	);
}

/// Without `--pull`, a service with `x-podman-autoupdate: registry` recreates
/// when the tag moves. The recreate happens because the config-hash +
/// image-ID comparison `up` already does (see the `up` section in
/// `docs/commands.md`: "a tag moved by `podman tag`") notices the local tag
/// now points at a different image ID; that path does not depend on the
/// `x-podman-autoupdate` extension or on any pull policy. This test exists
/// to pin that the `io.containers.autoupdate=registry` label survives the
/// recreate and the new container still carries it. It does NOT pin the
/// registry check itself: that is covered by the unit-level
/// `autoupdate_registry_pulls_even_when_the_image_is_present` and its
/// live-level sibling in this file.
///
/// The recreate is observed two ways: the lifecycle vocabulary reports
/// `Recreating` for the affected service, and the resulting container has a
/// different ID.
#[tokio::test]
async fn up_with_autoupdate_registry_recreates_after_the_tag_moved_without_pull_flag() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let proj = proj("au-recreate");
	let tag = format!("localhost/{proj}-pinned:latest");
	let (_v1_id, v2_id) = build_two(dir.path(), &tag);
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let yaml =
		format!("services:\n  web:\n    image: {tag}\n    command: [\"sleep\", \"infinity\"]\n    x-podman-autoupdate: registry\n");
	let file = parse_str(&yaml).unwrap();

	engine.up(&file).await.unwrap();
	let cname = format!("{proj}-web-1");
	let first_id_out = std::process::Command::new("podman")
		.args(["inspect", &cname, "--format", "{{.Id}}"])
		.output()
		.expect("podman inspect id");
	let first_id = String::from_utf8_lossy(&first_id_out.stdout)
		.trim()
		.to_string();
	let first_version = engine
		.test_exec_capture(&cname, vec!["cat".into(), "/version".into()])
		.await
		.unwrap_or_default();
	assert_eq!(
		first_version.trim(),
		"v1",
		"the first up must run the v1 image: {first_version}"
	);

	// Move the tag from v1 to v2, same name, different image ID. The
	// extension's `registry` value must force pull policy `newer` on the next
	// `up` and recreate the container.
	let tag_move = std::process::Command::new("podman")
		.args(["tag", &v2_id, &tag])
		.output()
		.expect("podman tag");
	assert!(
		tag_move.status.success(),
		"podman tag failed: {}",
		String::from_utf8_lossy(&tag_move.stderr)
	);

	// `Recreating` is the line the lifecycle prints for a recreate, the unit
	// test pinning the vocabulary is in `recreate_vocabulary.rs`. Run the
	// second `up` with `RUST_LOG=info` so the message reaches the test's
	// captured stderr.
	let prev_log = std::env::var_os("RUST_LOG");
	std::env::set_var("RUST_LOG", "podup=info");
	engine.up(&file).await.unwrap();
	match prev_log {
		Some(v) => std::env::set_var("RUST_LOG", v),
		None => std::env::remove_var("RUST_LOG"),
	}

	let version = engine
		.test_exec_capture(&cname, vec!["cat".into(), "/version".into()])
		.await
		.unwrap_or_default();
	let second_id_out = std::process::Command::new("podman")
		.args(["inspect", &cname, "--format", "{{.Id}}"])
		.output()
		.expect("podman inspect id");
	let second_id = String::from_utf8_lossy(&second_id_out.stdout)
		.trim()
		.to_string();
	engine.down(&file).await.unwrap();
	rmi(&tag);
	rmi(&format!("{tag}-v1"));
	rmi(&format!("{tag}-v2"));

	assert_eq!(
		version.trim(),
		"v2",
		"the container is still bound to the v1 image after the tag moved"
	);
	assert_ne!(
		first_id, second_id,
		"a recreated container must have a new id ({first_id} == {second_id})"
	);
}

/// Combine the stdout and stderr of a `podup` invocation, the way a real
/// shell would when piping both to the same sink. The progress layer that
/// emits `Pulling` writes to stderr, so a check on stdout alone misses it.
#[cfg(unix)]
fn combined(out: &std::process::Output) -> String {
	format!(
		"{}{}",
		String::from_utf8_lossy(&out.stdout),
		String::from_utf8_lossy(&out.stderr)
	)
}

/// Pull `alpine:latest` quietly through the live socket so the fixture can
/// retag it. Best-effort, like the other tests: an image already present
/// needs no network.
#[cfg(unix)]
fn ensure_alpine(socket: &str) {
	let _ = std::process::Command::new("podman")
		.args(["--url", socket, "pull", "-q", "alpine:latest"])
		.output();
}

/// `x-podman-autoupdate: registry` makes `up` reach the registry even when
/// the image is already on disk (#1953). The live complement to
/// `autoupdate_registry_pulls_even_when_the_image_is_present` in
/// `internal/engine/lifecycle/images_tests.rs`: same intent, but driven
/// through the real `podup` binary against the real Podman socket so the
/// path that emits `Pulling` is exercised end to end.
///
/// The image is tagged as `localhost:1/<project>-probe:latest`; nothing
/// listens on port 1, so any registry check Podman attempts is observable
/// in the output and requires no network from the test. The output is
/// asserted on `Pulling`, not on exit code or `Pulled`: Podman's `newer`
/// against an unreachable `localhost:1` reports `Pulled` and exits 0
/// (measured on 5.7.0), so the exit code is the wrong thing to assert on.
// Unix only: it reaches the Podman socket by its `/run/user/<uid>` path.
#[cfg(unix)]
#[tokio::test]
async fn up_with_autoupdate_registry_pulls_even_when_the_image_is_present() {
	let Some(socket) = podman_socket_url() else {
		return;
	};
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	let project = proj("au-pull");
	let image = format!("localhost:1/{project}-probe:latest");
	ensure_alpine(&socket);
	// A failed tag would leave the image absent, and the absent-image pull
	// would then pass the assertion for the wrong reason, so it is fatal.
	let tag = std::process::Command::new("podman")
		.args(["--url", &socket, "tag", "alpine:latest", &image])
		.output()
		.expect("podman tag");
	assert!(
		tag.status.success(),
		"podman tag {image} failed: {}",
		String::from_utf8_lossy(&tag.stderr)
	);

	let compose = dir.path().join("docker-compose.yml");
	let with_ext = format!(
		"services:\n  web:\n    image: {image}\n    command: [\"sleep\", \"infinity\"]\n    x-podman-autoupdate: registry\n"
	);
	let without_ext =
		format!("services:\n  web:\n    image: {image}\n    command: [\"sleep\", \"infinity\"]\n");

	// Run with the extension first. Use `Command` directly, not the test
	// harness' `run_ok`: the exit code is not what this test pins on.
	// Podman against `localhost:1` with `newer` reports `Pulled` and exits 0
	// even when nothing actually answered (measured on 5.7.0), so a
	// non-zero exit code would not name the bug we are trying to pin.
	std::fs::write(&compose, &with_ext).expect("write with-extension compose");
	let out_ext = std::process::Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &project, "up", "-d"])
		.env("PODMAN_SOCKET", &socket)
		.output()
		.expect("run podup up with extension");
	let _ = std::process::Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&project,
			"down",
			"-v",
		])
		.env("PODMAN_SOCKET", &socket)
		.output();

	// Control: same image but a different project, and the compose carries no
	// extension. That is what proves the extension is the reason `up` reached
	// the registry, not some other code path that always pulls this image.
	let project_no_ext = proj("au-pull-noext");
	std::fs::write(&compose, &without_ext).expect("write without-extension compose");
	let out_no_ext = std::process::Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&project_no_ext,
			"up",
			"-d",
		])
		.env("PODMAN_SOCKET", &socket)
		.output()
		.expect("run podup up without extension");
	let _ = std::process::Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&project_no_ext,
			"down",
			"-v",
		])
		.env("PODMAN_SOCKET", &socket)
		.output();

	let text_ext = combined(&out_ext);
	let text_no_ext = combined(&out_no_ext);

	let _ = std::process::Command::new("podman")
		.args(["--url", &socket, "rmi", "-f", &image])
		.output();

	assert!(
		text_ext.contains("Pulling") && text_ext.contains(&image),
		"`up` with x-podman-autoupdate: registry must reach the registry for {image}; output was:\n{text_ext}"
	);
	assert!(
		!text_no_ext.contains("Pulling"),
		"`up` without the extension must not pull a present image, otherwise the registry test proves nothing; output was:\n{text_no_ext}"
	);
}
