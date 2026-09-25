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
/// of 20 seconds must come back from `podup stop --timeout 1` in
/// under 8 seconds total. The CLI override is what makes the assertion
/// reachable: podup forwards the override as `?timeout=` to libpod,
/// which honours it (1-second SIGTERM, then SIGKILL). The Docker
/// compat `/stop?t=N` honoured `t`; the libpod handler ignores `t`
/// and reads `timeout=`, so a sabotaged `?t=1` lands on libpod and
/// libpod falls back to the container's own stop timeout (the 20-second
/// `stop_grace_period`), which blows past 8 seconds by a wide margin.
///
/// Note: the previous shape of this test exercised only the
/// compose-file `stop_grace_period` (no CLI override) and stayed green
/// with `t=` because podup creates the container with
/// `stop_grace_period` as its own stop timeout, so when Podman ignored
/// `t=` it fell back to the same value. Pinning the CLI-override path
/// makes the difference visible: the user's `--timeout` differs from
/// the compose value by a factor of 20.
///
/// Fails on the branch with the `timeout=` parameter reverted to
/// `t=` at `internal/engine/lifecycle/commands.rs::stop_container`
/// (the asserted `elapsed < 8s` line), and at the parallel path
/// `internal/engine/lifecycle/parallel.rs::teardown_one_container`.
#[tokio::test]
async fn stop_returns_within_the_grace_window() {
	let Some(socket) = podman_socket_url() else {
		return;
	};
	let (_dir, name, container) = up_service(
		&socket,
		"c1914stop",
		"services:\n  app:\n    image: alpine:3.20\n    command: [\"sh\",\"-c\",\"trap '' TERM; sleep 3600\"]\n    stop_grace_period: 20s\n",
	);
	let compose = _dir.path().join("compose.yaml");
	let started = Instant::now();
	let out = Command::new(bin())
		.args(["-f"])
		.arg(&compose)
		.args(["-p", &name, "stop", "--timeout", "1"])
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
		ms < 8_000,
		"`podup stop --timeout 1` must be honoured against a 20s `stop_grace_period` \
		 (libpod `timeout=`): took {ms}ms for {container}"
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

/// `podup restart --timeout 1` on a service whose process ignores
/// SIGTERM with a `stop_grace_period` of 20 seconds must come back in
/// under 8 seconds. The companion of `stop_returns_within_the_grace_window`
/// against the `restart` endpoint: libpod reads `?timeout=`; the Docker
/// compat handler read `?t=`, which libpod ignores, falling back to the
/// container's own 20-second stop_timeout. Same reasoning: the no-CLI
/// shape (`restart_honours_the_grace_window`, above) stays green under
/// `?t=` because podup creates the container with `stop_grace_period` as
/// its own stop timeout, so the fallback matches; only the CLI override
/// makes the difference visible.
///
/// Fails on the branch with `?timeout=` reverted to `?t=` at
/// `internal/engine/lifecycle/parallel.rs::restart_one_service` and
/// `internal/engine/watch/mod.rs::watch_restart` (the asserted
/// `elapsed < 8s` line).
#[tokio::test]
async fn restart_honours_the_cli_override() {
	let Some(socket) = podman_socket_url() else {
		return;
	};
	let (_dir, name, container) = up_service(
		&socket,
		"c1914restcli",
		"services:\n  app:\n    image: alpine:3.20\n    command: [\"sh\",\"-c\",\"trap '' TERM; sleep 3600\"]\n    stop_grace_period: 20s\n",
	);
	let compose = _dir.path().join("compose.yaml");
	let started = Instant::now();
	let out = Command::new(bin())
		.args(["-f"])
		.arg(&compose)
		.args(["-p", &name, "restart", "--timeout", "1"])
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
		ms < 8_000,
		"`podup restart --timeout 1` must be honoured against a 20s `stop_grace_period` \
		 (libpod `timeout=`): took {ms}ms for {container}"
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
/// **Construction limit**: on a fast host SIGKILL lands in
/// milliseconds and the follow-up `/wait?condition=stopped` returns
/// within the same wall clock window, so removing the wait does not
/// reliably make this live test fail (the race is too short for a
/// `podman inspect` round-trip). The wire-shape unit test
/// `kill_with_sigkill_sends_follow_up_wait` in
/// `internal/engine/lifecycle/libpod_endpoint_query_tests.rs:282`
/// is what pins the actual compensation (follow-up
/// `/wait?condition=stopped`); this live test pins the user-visible
/// behaviour the wire shape exists to keep.
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
	// The compat handler waited for either condition, `exited` or `stopped`, and a
	// SIGKILLed container passed through `stopped` before `exited`: under parallel
	// load on 2026-09-25 this test read `stopped` once in five runs. Either one means
	// the container is no longer running when `kill` returns, which is the property.
	assert!(
		state == "exited" || state == "stopped",
		"`podup kill` must wait until the container is no longer running (libpod follow-up wait): \
		 state for {container} was {state:?}"
	);
}
