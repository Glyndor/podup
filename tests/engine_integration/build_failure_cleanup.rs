//! #1910: a `podup build` that fails must not leave a buildah working
//! container behind. The absolute-form request line that the old
//! `build_request` wrote landed `POST http://localhost/v5.0.0/libpod/build
//! HTTP/1.1` on the wire, and Podman 5.7.0 did not clean up its buildah
//! working container on that failure shape. Switching to origin form
//! closed it (4 of 4 runs leaked before, 0 of 2 after, measured on
//! 2026-09-24).
//!
//! The test drives the built `podup` binary (via
//! `env!("CARGO_BIN_EXE_podup")`) rather than the in-process `Engine`,
//! because the in-process path did not reproduce the leak when measured.
//! On 2026-09-24, against Podman 5.7.0 with the same Containerfile
//! shape and the absolute-form request line restored, the CLI binary
//! leaked the buildah working container on 2 of 2 runs while the
//! in-process `engine.build_all` path leaked on 0 of 3. The test runs
//! a Containerfile whose `RUN exit 1` makes the build fail, asserts
//! the exit code is non-zero AND that the daemon's error names the
//! `RUN exit 1` step (so a future change that turns a build failure
//! into a silent success cannot green out this test by accident), and
//! then asserts no external (buildah working) container references any
//! image this build produced. Identification is by
//! `podup.project=<project>` and `podup.service=app`, the two keys
//! `internal/engine/build/service.rs` stamps on every layer of the
//! build via the build query's `layerLabel=` parameter. Other tests
//! building on the same socket may also be running, so the harness
//! scopes the assertion to images this build actually produced.
use super::*;
use crate::build_labels::TestImages;
use std::process::Command;

/// Locate the Podman socket the engine talks to, the same way
/// `build_labels.rs` does. The CLI's own storage root is often
/// different from the socket's, so plain `podman ps` queries the wrong
/// store; the CLI's `--url` flag forwards the request to the socket
/// instead, and that is what the live test inspects.
fn podman_socket_url() -> Option<String> {
	for path in [
		format!("/run/user/{}/podman/podman.sock", unsafe { libc::getuid() }),
		"/run/podman/podman.sock".to_string(),
	] {
		if std::path::Path::new(&path).exists() {
			return Some(format!("unix://{path}"));
		}
	}
	None
}

fn labeled_image_ids(socket: &str, project: &str) -> std::collections::HashSet<String> {
	let out = std::process::Command::new("podman")
		.args([
			"--url",
			socket,
			"images",
			"-a",
			"-q",
			"--no-trunc",
			"--filter",
			&format!("label=podup.project={project}"),
		])
		.output()
		.expect("podman images --filter label");
	if !out.status.success() {
		panic!(
			"`podman --url {socket} images -a -q --no-trunc --filter label=podup.project={project}` \
			 exited {}: {}",
			out.status,
			String::from_utf8_lossy(&out.stderr),
		);
	}
	String::from_utf8_lossy(&out.stdout)
		.lines()
		.map(str::trim)
		.filter(|s| !s.is_empty())
		// `podman images -q --no-trunc` prints `sha256:<hex>` while `ps --format {{.ImageID}}`
		// prints the bare hex, so the prefix is dropped or the two sets never meet.
		.map(|id| id.strip_prefix("sha256:").unwrap_or(id).to_string())
		.collect()
}

/// Every external (buildah working) container visible to the socket,
/// paired with the image id it was created from. The shape is the
/// one `podman ps --help` documents on this machine: `podman ps -a
/// --external --format "{{.ID}} {{.ImageID}}"`. Empty when no working
/// containers exist, which is the steady state this test pins.
fn external_container_image_ids(socket: &str) -> Vec<(String, String)> {
	let out = std::process::Command::new("podman")
		.args([
			"--url",
			socket,
			"ps",
			"-a",
			"--external",
			"--no-trunc",
			"--noheading",
			"--format",
			"{{.ID}} {{.ImageID}}",
		])
		.output()
		.expect("podman ps -a --external");
	if !out.status.success() {
		panic!(
			"`podman --url {socket} ps -a --external` exited {}: {}",
			out.status,
			String::from_utf8_lossy(&out.stderr),
		);
	}
	String::from_utf8_lossy(&out.stdout)
		.lines()
		.map(str::trim)
		.filter(|s| !s.is_empty())
		.filter_map(|line| {
			let mut parts = line.split_whitespace();
			Some((parts.next()?.to_string(), parts.next()?.to_string()))
		})
		.collect()
}

/// Remove every external container whose id is in `ids`. Best-effort:
/// any error is logged on stderr and swallowed so a single failed
/// reap does not derail the rest of the cleanup or the assertion that
/// follows.
fn remove_external_containers(socket: &str, ids: &[String]) {
	if ids.is_empty() {
		return;
	}
	let mut args: Vec<String> = vec!["--url".into(), socket.into(), "rm".into(), "-f".into()];
	args.extend(ids.iter().cloned());
	let out = std::process::Command::new("podman").args(&args).output();
	match out {
		Ok(o) if o.status.success() => {}
		Ok(o) => eprintln!(
			"build_failure_cleanup: `podman rm -f` for {} ids exited {}: {}",
			ids.len(),
			o.status,
			String::from_utf8_lossy(&o.stderr)
		),
		Err(e) => eprintln!("build_failure_cleanup: failed to spawn `podman rm`: {e}"),
	}
}

/// A failing `podup build` against Podman 5.7.0 must not leave a
/// buildah working container behind. The build's intermediate images
/// are tagged with `podup.project=<project>` via the `layerLabel=`
/// parameter (`internal/engine/build/service.rs`), so identification
/// does not depend on global image counts: parallel tests can be
/// building on the same socket without making this one false-pass.
///
/// The test runs the built `podup` binary, not `Engine::build_all`,
/// because the in-process path did not reproduce the leak when
/// measured: on 2026-09-24 the binary leaked 2 of 2 runs with the
/// absolute-form line restored while `Engine::build_all` leaked 0 of
/// 3 on the same machine. Driving the binary is what makes the
/// assertion bind to the property the bug actually has.
#[tokio::test]
async fn a_failing_build_leaves_no_buildah_working_container() {
	// `client` is fetched only to honour the same `podman()` reachability
	// gate the other engine-integration tests use; the test body talks to
	// Podman through the `podup` binary, not through this client.
	let _client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let Some(socket) = podman_socket_url() else {
		return;
	};
	let dir = tempfile::tempdir().unwrap();
	let project = proj("bfail");
	let service = "app";

	// Containerfile that runs once successfully and then fails. The
	// successful `RUN` is what produces an intermediate image carrying
	// `podup.project=<project>`: the working container a leaking build
	// leaves behind uses that image as its rootfs, so without it there
	// is nothing for the assertion to identify the leak with. The
	// marker is process-unique so two test runs back-to-back cannot
	// match each other's layer cache.
	let run_marker = format!(
		"echo bf-{}-{} > /x",
		std::process::id(),
		std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap()
			.as_nanos()
	);
	let dockerfile = format!("FROM docker.io/library/alpine:3.20\nRUN {run_marker}\nRUN exit 1\n");
	fs::write(dir.path().join("Containerfile"), dockerfile).unwrap();
	// The compose carries the project name (`name: <project>`) so the
	// `podup.project=<project>` label the engine stamps onto every
	// layer is reachable without a separate `-p` flag, and one service
	// `app` with `build: .` covers the shape the harness was specified
	// to build.
	let compose = format!(
		"name: {project}\nservices:\n  {service}:\n    build:\n      context: .\n    \
		 command: [\"sleep\", \"infinity\"]\n"
	);
	fs::write(dir.path().join("compose.yml"), compose).unwrap();

	let build_output = Command::new(env!("CARGO_BIN_EXE_podup"))
		.arg("build")
		.current_dir(dir.path())
		.output()
		.expect("run podup build");
	let stderr = String::from_utf8_lossy(&build_output.stderr);
	// Two assertions, not one: the exit status proves the build
	// failed, the stderr string proves it failed for the `RUN exit 1`
	// reason we set up. A future change that swallowed the build
	// error and exited 0 cannot pass the first assertion, and a
	// regression that turned a different step into the failing one
	// (network error, OOM, missing context) cannot pass the second.
	assert!(
		!build_output.status.success(),
		"the `RUN exit 1` build must exit non-zero; exited 0 with stderr:\n{stderr}"
	);
	assert!(
		stderr.contains("RUN exit 1"),
		"the build must fail on the `RUN exit 1` step we wrote, not on something else; \
		 stderr was:\n{stderr}"
	);

	// Every image this build produced carries `podup.project=<project>`
	// (the build's `layerLabel`), the intermediate stages included, and the
	// project name is unique to this run, so the label alone scopes the
	// check to this build even while other tests build on the same socket.
	let ours = labeled_image_ids(&socket, &project);

	// Every external (buildah working) container that references one
	// of this build's images is a leak. Reading `.ImageID` matches what
	// the buildah working container was rooted at; a leak on the
	// absolute-form request line showed up there on every run of the
	// binary, with the leaked container's root image carrying
	// `podup.project=<project>`.
	let leaked: Vec<(String, String)> = external_container_image_ids(&socket)
		.into_iter()
		.filter(|(_id, image_id)| ours.contains(image_id))
		.collect();

	// Reap any leak the harness found, so a failing run leaves no
	// debris behind for the next test to inherit. The leak ids are
	// the exact `buildah` ids the test would print on failure, so an
	// operator can re-run the reap by hand from the same message.
	let leaked_ids: Vec<String> = leaked.iter().map(|(id, _)| id.clone()).collect();
	remove_external_containers(&socket, &leaked_ids);

	// Clean up the build's images the same way the sibling label
	// tests do, so a failing test does not pin images on disk.
	let _images = TestImages::new(&project, service, &socket);

	assert!(
		leaked.is_empty(),
		"the failing build leaked {} buildah working container(s) rooted at \
		 this build's images: {:?}",
		leaked.len(),
		leaked,
	);
}
