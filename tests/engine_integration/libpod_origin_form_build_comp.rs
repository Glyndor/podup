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
/// The two builds run back-to-back with no teardown between them,
/// because that is the only shape that pins the docker-distribution
/// `outputformat=` parameter on every Podman version that runs the
/// suite. Podman 5.8.1 and 6.1.2 (the CI lane, a nested-virt runner)
/// drop the intermediate layers when an image is removed with
/// `podup down --rmi local`, so a teardown between the two builds
/// gave the second build nothing to hit and the `Using cache`
/// assertion could not fire. Podman 5.7.0 (local socket) and 6.0.1
/// kept the layers across `rmi`, which is why the local run stayed
/// green and the CI lane went red. The docker-distribution manifest
/// is the user-visible behaviour the `outputformat=` parameter
/// exists to keep, and the only way to keep that pin across the
/// version spread is to leave the layers between the two builds and
/// clean up at the end.
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

	// The docker compat build handler passed every `t=` and `/images/{}/tag`
	// argument through `NormalizeToDockerHub`, so `proj/c1914:1` (an
	// org-scoped short name) used to land as `docker.io/proj/c1914:1`.
	// On the libpod path `NormalizeToDockerHub` short-circuits, so the
	// image lands as `localhost/proj/c1914:1` instead. The wire-level
	// pin lives in `internal::libpod::normalize_tests`; this assertion
	// reads the user-visible result so a regression there is caught at
	// the boundary the user sees (`podman image ls`, `ps IMAGE`,
	// `events image=...`).
	let first_repo_tags = podman_cmd(
		&socket,
		&[
			"image",
			"inspect",
			"proj/c1914:1",
			"--format",
			"{{.RepoTags}}",
		],
	);

	// No inter-build teardown. Podman 5.8.1 / 6.1.2 (the CI lane)
	// remove the intermediate layers alongside the image, which would
	// force the second build to start cold and miss the `Using cache`
	// path that this test exists to pin. Podman 5.7.0 / 6.0.1 keep
	// the layers, which is why the local run stayed green before the
	// CI lane went red. The final `podup down --rmi local` below
	// cleans up regardless of which version ran the test.
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
	let second_repo_tags = podman_cmd(
		&socket,
		&[
			"image",
			"inspect",
			"proj/c1914:1",
			"--format",
			"{{.RepoTags}}",
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
	// `podman image inspect --format '{{.RepoTags}}'` renders the slice
	// as Go does, with each tag in square brackets and quotes. A
	// substring match for the canonical entry is enough to pin the
	// normalisation: `docker.io/proj/c1914:1` is what `NormalizeToDockerHub`
	// produced on the compat path, and what `normalize_image_reference`
	// reproduces here.
	let canonical_tag = "docker.io/proj/c1914:1";
	assert!(
		first_repo_tags.contains(canonical_tag),
		"`podup build` (1) must land the image under the docker.io canonical name \
		 (`NormalizeToDockerHub` on the compat path applied this), not the libpod \
		 `localhost/...` default: got {first_repo_tags:?}"
	);
	assert!(
		second_repo_tags.contains(canonical_tag),
		"`podup build` (2) must land the image under the docker.io canonical name \
		 (`NormalizeToDockerHub` on the compat path applied this), not the libpod \
		 `localhost/...` default: got {second_repo_tags:?}"
	);
}
