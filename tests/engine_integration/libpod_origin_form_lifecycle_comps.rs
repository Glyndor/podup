//! #1914 compensations: lifecycle endpoints the libpod switch disturbed.
//!
//! Each test pins one of the four lifecycle behaviours: the libpod
//! handler reads a different query key or returns before the
//! container reaches the expected state, and podup's user-visible
//! behaviour (the docker-compat shape) is the contract these tests are
//! here to keep honest. Every test runs against the same Podman 5.7.0
//! socket the build/lifecycle unit tests already assume, and skips
//! cleanly when no daemon is reachable.
//!
//! Each test fails with its compensation removed (the unit tests in
//! `engine::lifecycle::libpod_endpoint_query_tests` pin the wire
//! shape; these tests pin the user-visible behaviour the wire shape
//! exists to keep). The expected failing line, when the compensation
//! is reverted, is the one the comment on the test names.

#[allow(unused_imports)]
use super::*;
use std::process::Command;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Compensation 1: `POST /containers/{}/stop` must read `timeout=`, not `t=`
// ---------------------------------------------------------------------------

/// A service whose process ignores SIGTERM with a `stop_grace_period`
/// of 2 seconds must come back from `podup stop` in under 6 seconds
/// total. The Docker compat `/stop?t=N` honoured `t`; the libpod
/// `/stop` ignores `t` and reads `timeout=`, so an unchanged `t=2`
/// query lands on libpod and libpod stops ignoring the grace period,
/// which means it falls back to the container's own stop timeout (or
/// to the daemon default of 10 seconds when the container has none
/// configured). The 6-second upper bound catches the libpod
/// regression: a 2-second grace under compensation returns in
/// roughly 2 seconds, and the docker-compat fallback (10 seconds)
/// blows past 6 by a wide margin.
///
/// Fails on the branch with the `timeout=` parameter reverted to
/// `t=` at `internal/engine/lifecycle/commands.rs::stop_container`
/// (the asserted `elapsed < 6s` line), and at the parallel path
/// `internal/engine/lifecycle/parallel.rs::teardown_one_container`.
#[tokio::test]
async fn stop_returns_within_the_grace_window() {
	let Some(socket) = podman_socket_url() else {
		return;
	};
	let (_dir, name, container) = up_service(
		&socket,
		"c1914stop",
		"services:\n  app:\n    image: alpine:3.20\n    command: [\"sh\",\"-c\",\"trap '' TERM; sleep 3600\"]\n    stop_grace_period: 2s\n",
	);
	let compose = _dir.path().join("compose.yaml");
	let started = Instant::now();
	let out = Command::new(bin())
		.args(["-f"])
		.arg(&compose)
		.args(["-p", &name, "stop"])
		.env("PODMAN_SOCKET", &socket)
		.output()
		.expect("run podup stop");
	let ms = elapsed_ms(started);
	assert!(
		out.status.success(),
		"`podup stop` failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	down(&socket, &_dir, &name);
	assert!(
		ms < 6_000,
		"`podup stop` must honour the 2s grace window (libpod `timeout=`): took {ms}ms for {container}"
	);
}

// ---------------------------------------------------------------------------
// Compensation 2: `POST /containers/{}/restart` must read `timeout=`, not `t=`
// ---------------------------------------------------------------------------

/// `podup restart` on a service whose process ignores SIGTERM with a
/// `stop_grace_period` of 3 seconds must take at least 2.5 seconds
/// before the container is running again. The libpod handler ignores
/// `t=` and defaults `timeout=` to 0 when absent, which is an
/// immediate SIGKILL: the SIGTERM the container is configured to
/// ignore never lands, and the container comes back in well under a
/// second. The 2.5-second lower bound catches the libpod regression.
///
/// Fails on the branch with the `timeout=` parameter reverted to
/// `t=` at `internal/engine/lifecycle/parallel.rs::restart_one_service`
/// and `internal/engine/watch/mod.rs::watch_restart` (the asserted
/// `elapsed >= 2.5s` line).
#[tokio::test]
async fn restart_honours_the_grace_window() {
	let Some(socket) = podman_socket_url() else {
		return;
	};
	let (_dir, name, container) = up_service(
		&socket,
		"c1914restart",
		"services:\n  app:\n    image: alpine:3.20\n    command: [\"sh\",\"-c\",\"trap '' TERM; sleep 3600\"]\n    stop_grace_period: 3s\n",
	);
	let compose = _dir.path().join("compose.yaml");
	let started = Instant::now();
	let out = Command::new(bin())
		.args(["-f"])
		.arg(&compose)
		.args(["-p", &name, "restart"])
		.env("PODMAN_SOCKET", &socket)
		.output()
		.expect("run podup restart");
	let ms = elapsed_ms(started);
	assert!(
		out.status.success(),
		"`podup restart` failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	down(&socket, &_dir, &name);
	assert!(
		ms >= 2_500,
		"`podup restart` must honour the 3s grace window (libpod `timeout=`): took {ms}ms for {container}, expected >= 2500ms"
	);
}

// ---------------------------------------------------------------------------
// Compensation 3: `DELETE /containers/{}?volumes=true`, not `v=true`
// ---------------------------------------------------------------------------

/// `podup down -v` on a service with `volumes: ["/data"]` (an
/// anonymous volume) must remove that anonymous volume. The libpod
/// delete handler reads `volumes=`; the Docker compat handler reads
/// `v=`, which the libpod handler ignores, so a `down -v` against
/// libpod without `volumes=` reclaims nothing and the test fails
/// when the volume id remains queryable.
///
/// Fails on the branch with `v=true` left in `container_rm_path`
/// at `internal/engine/lifecycle/mod.rs` (the asserted
/// `volumes.count == 0` line).
#[tokio::test]
async fn down_v_removes_anonymous_volumes() {
	let Some(socket) = podman_socket_url() else {
		return;
	};
	let (_dir, name, container) = up_service(
		&socket,
		"c1914downv",
		"services:\n  app:\n    image: alpine:3.20\n    command: [\"sleep\", \"3600\"]\n    volumes:\n      - \"/data\"\n",
	);

	// Capture the anonymous volume id the running container owns.
	let mounts = podman_cmd(
		&socket,
		&["inspect", &container, "--format", "{{json .Mounts}}"],
	);
	let volume_name: String = serde_json::from_str::<serde_json::Value>(&mounts)
		.ok()
		.and_then(|v| {
			v.as_array()
				.and_then(|arr| arr.first())
				.and_then(|m| m.get("Name"))
				.and_then(|n| n.as_str())
				.map(str::to_string)
		})
		.unwrap_or_default();
	assert!(
		!volume_name.is_empty(),
		"the container did not own an anonymous volume: {mounts}"
	);

	down(&socket, &_dir, &name);

	// `podman volume exists` prints the volume name on success and
	// exits non-zero when the volume is gone. The non-zero exit is the
	// expected end state; an exit 0 would mean the libpod `volumes=`
	// compensation is missing and the volume leaked.
	let exists = Command::new("podman")
		.args(["--url", &socket, "volume", "exists", &volume_name])
		.output()
		.expect("podman volume exists");
	assert!(
		!exists.status.success(),
		"`podup down -v` left the anonymous volume `{volume_name}` behind (libpod `volumes=true`)"
	);
}

// ---------------------------------------------------------------------------
// Compensation 4: `kill SIGKILL` waits for the container to exit
// ---------------------------------------------------------------------------

/// `podup kill app` (SIGKILL by default) must return only after the
/// container is in `exited` state. The Docker compat `/kill` handler
/// blocks on SIGKILL until the container exits; the libpod handler
/// replies immediately. Without the follow-up `/wait?condition=stopped`
/// a script that polled `podup kill` and then read the container
/// state would see `running` for a few hundred ms, which is what
/// every caller that relied on the compat handler's semantics
/// observed before the compensation.
///
/// Fails on the branch with the follow-up `wait_after_kill` removed
/// from `internal/engine/lifecycle/parallel.rs::kill_one_service`
/// (the asserted `state == exited` line).
#[tokio::test]
async fn kill_returns_only_after_the_container_is_exited() {
	let Some(socket) = podman_socket_url() else {
		return;
	};
	let (_dir, name, container) = up_service(
		&socket,
		"c1914kill",
		"services:\n  app:\n    image: alpine:3.20\n    command: [\"sleep\", \"3600\"]\n",
	);
	let compose = _dir.path().join("compose.yaml");
	let out = Command::new(bin())
		.args(["-f"])
		.arg(&compose)
		.args(["-p", &name, "kill"])
		.env("PODMAN_SOCKET", &socket)
		.output()
		.expect("run podup kill");
	assert!(
		out.status.success(),
		"`podup kill` failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);

	let state = podman_cmd(
		&socket,
		&["inspect", &container, "--format", "{{.State.Status}}"],
	);
	down(&socket, &_dir, &name);
	assert_eq!(
		state, "exited",
		"`podup kill` must wait for the container to be exited (libpod follow-up wait): \
		 state for {container} was {state:?}"
	);
}
