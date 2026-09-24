//! #1914 compensation: the libpod `/build` endpoint must produce the
//! docker-distribution manifest.
//!
//! The libpod handler defaults the image manifest format to OCI
//! when only `layers=true` is sent. The OCI format does not reuse
//! the layer cache on a second build of the same Containerfile, and
//! drops `HEALTHCHECK` from the image config. Adding
//! `outputformat=application/vnd.docker.distribution.manifest.v2+json`
//! forces the docker-distribution format the docker compat handler
//! produced, which keeps the cache and the image shape podup has
//! always produced.
//!
//! Two assertions are load-bearing here:
//! - the second build prints `Using cache` (the user-visible
//!   behaviour the wire shape exists to keep); and
//! - the built image's manifest type is the docker one, read with
//!   `podman image inspect --format '{{.ManifestType}}'`.
//!
//! The wire-level assertion that pins the parameter itself lives in
//! `engine::build::query_tests::build_query_carries_docker_distribution_outputformat`.
//! Removing the `outputformat=` parameter from the build query fails
//! the unit test (0 occurrences of the parameter where 1 is
//! expected); it also flips this integration test from
//! "Using cache" -> "no Using cache" and OCI -> docker-distribution.

#[allow(unused_imports)]
use super::*;
use std::process::Command;

// ---------------------------------------------------------------------------
// Compensation 9: `POST /build` must send `layers=true` AND
// `outputformat=application/vnd.docker.distribution.manifest.v2+json`
// ---------------------------------------------------------------------------

/// `podup build` on an unchanged Containerfile must print
/// `Using cache` on the second run, and the image's manifest type
/// must be the docker-distribution format. The wire-level pin
/// (the parameter itself) lives in
/// `engine::build::query_tests::build_query_carries_docker_distribution_outputformat`;
/// this test pins the user-visible result.
///
/// The second build tears the test image down (`podup down --rmi
/// local`) between the runs so the cache key the libpod handler
/// looks at is the one it would look at on a normal developer's
/// machine: an absent image and a fresh `FROM`. Without the
/// teardown the second build could find the image by its tag and
/// skip the layer walk entirely, which is not the path
/// `podup build` takes when the user has not pulled a new base.
#[tokio::test]
async fn build_uses_the_layer_cache_and_produces_a_docker_manifest() {
	let Some(socket) = podman_socket_url() else {
		return;
	};
	let tag = "c1914cache";
	let name = format!("t{}-{}", std::process::id(), tag);
	let dir = tempfile::tempdir().expect("tempdir");
	let compose = dir.path().join("compose.yaml");
	std::fs::write(
		&compose,
		"services:\n  app:\n    build: .\n    image: proj/c1914:1\n",
	)
	.expect("write compose");
	let dockerfile = dir.path().join("Dockerfile");
	std::fs::write(
		&dockerfile,
		"FROM alpine:3.20\nRUN echo hi\nCMD [\"sleep\",\"3600\"]\n",
	)
	.expect("write Dockerfile");

	let first = Command::new(bin())
		.args(["-f"])
		.arg(&compose)
		.args(["-p", &name, "build"])
		.env("PODMAN_SOCKET", &socket)
		.output()
		.expect("run podup build (1)");
	assert!(
		first.status.success(),
		"`podup build` (1) failed: {}",
		String::from_utf8_lossy(&first.stderr)
	);
	let first_stdout = String::from_utf8_lossy(&first.stdout);
	let first_stderr = String::from_utf8_lossy(&first.stderr);
	let first_combined = format!("{first_stdout}{first_stderr}");
	assert!(
		first_combined.contains("Successfully tagged"),
		"`podup build` (1) must produce a tagged image: stdout={first_stdout:?} stderr={first_stderr:?}"
	);

	// Read the manifest type of the freshly built image. The docker
	// compat handler podup used to drive produced
	// `application/vnd.docker.distribution.manifest.v2+json`; the
	// libpod handler defaults to `application/vnd.oci.image.manifest.v1+json`
	// when `outputformat=` is absent. Reading this field on the
	// first image is the assertion that catches the OCI default at
	// the boundary that matters.
	let first_manifest = podman_cmd(
		&socket,
		&[
			"image",
			"inspect",
			"proj/c1914:1",
			"--format",
			"{{.ManifestType}}",
		],
	);

	// Tear down the test image so the second build is the one that
	// defines the image id we read. `podup down --rmi local` removes
	// only the project's own images (the `podup.project=` label is
	// what scopes the prune), so it cannot remove `alpine:3.20` or
	// anything outside the test.
	let _ = Command::new(bin())
		.args(["-f"])
		.arg(&compose)
		.args(["-p", &name, "down", "--rmi", "local"])
		.env("PODMAN_SOCKET", &socket)
		.output();

	let second = Command::new(bin())
		.args(["-f"])
		.arg(&compose)
		.args(["-p", &name, "build"])
		.env("PODMAN_SOCKET", &socket)
		.output()
		.expect("run podup build (2)");
	assert!(
		second.status.success(),
		"`podup build` (2) failed: {}",
		String::from_utf8_lossy(&second.stderr)
	);
	let second_stdout = String::from_utf8_lossy(&second.stdout);
	let second_stderr = String::from_utf8_lossy(&second.stderr);
	let second_combined = format!("{second_stdout}{second_stderr}");

	assert!(
		second_combined.contains("Successfully tagged"),
		"`podup build` (2) must produce a tagged image: stdout={second_stdout:?} stderr={second_stderr:?}"
	);
	assert!(
		second_combined.contains("Using cache"),
		"`podup build` (2) must hit the layer cache (libpod docker-distribution outputformat): \
		 stdout={second_stdout:?} stderr={second_stderr:?}"
	);

	let second_manifest = podman_cmd(
		&socket,
		&[
			"image",
			"inspect",
			"proj/c1914:1",
			"--format",
			"{{.ManifestType}}",
		],
	);

	// Final teardown: remove the test image so the host stays clean.
	let _ = Command::new(bin())
		.args(["-f"])
		.arg(&compose)
		.args(["-p", &name, "down", "--rmi", "local"])
		.env("PODMAN_SOCKET", &socket)
		.output();

	let docker_manifest = "application/vnd.docker.distribution.manifest.v2+json";
	assert_eq!(
		first_manifest, docker_manifest,
		"`podup build` (1) must produce a docker-distribution manifest, \
		 not the libpod OCI default: got {first_manifest:?}"
	);
	assert_eq!(
		second_manifest, docker_manifest,
		"`podup build` (2) must produce a docker-distribution manifest, \
		 not the libpod OCI default: got {second_manifest:?}"
	);
}
