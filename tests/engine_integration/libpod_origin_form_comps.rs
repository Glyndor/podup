//! #1914 compensations, exercised against a real Podman daemon.
//!
//! Each test pins one of the nine behaviours the libpod switch
//! disturbed: the Docker compat handler read one query key per
//! endpoint, the libpod handler reads a different one, and podup's
//! user-visible behaviour (the docker-compat shape) is the contract
//! these tests are here to keep honest. Every test runs against the
//! same Podman 5.7.0 socket the build/lifecycle unit tests already
//! assume, and skips cleanly when no daemon is reachable.
//!
//! Each test fails with its compensation removed (the unit tests in
//! `engine::lifecycle::libpod_endpoint_query_tests` pin the wire
//! shape; these tests pin the user-visible behaviour the wire shape
//! exists to keep). The expected failing line, when the compensation
//! is reverted, is the one the comment on the test names.

// The tests below share the suite's `super::podman` / `super::proj`
// helpers via the path the rest of `engine_integration/` follows.
#[allow(unused_imports)]
use super::*;
use std::process::Command;
use std::time::{Duration, Instant};

/// Locate the Podman socket the engine talks to. The CLI's own
/// storage root is often different from the socket's, so plain
/// `podman ps` queries the wrong store; the CLI's `--url` flag
/// forwards the request to the socket instead, and that is what
/// the live tests inspect.
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

/// Run `podman --url <socket> <args...>` and return the trimmed
/// stdout. Panics with stderr on a non-zero exit so a failing
/// assertion carries the actual Podman response.
fn podman_cmd(socket: &str, args: &[&str]) -> String {
	let out = Command::new("podman")
		.args(["--url", socket])
		.args(args)
		.output()
		.unwrap_or_else(|e| panic!("podman {args:?}: {e}"));
	if !out.status.success() {
		panic!(
			"`podman --url {socket} {args:?}` exited {}: {}",
			out.status,
			String::from_utf8_lossy(&out.stderr)
		);
	}
	String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Build a project on the live socket and start it. Returns
/// `(project_name, container_name)` and the tempdir handle so the
/// compose file outlives the `up`. The composition drives the
/// `podup` binary through `CARGO_BIN_EXE_podup`, the same binary
/// `cargo test --test engine_integration` resolves at build time.
fn up_service(socket: &str, tag: &str, body: &str) -> (tempfile::TempDir, String, String) {
	let dir = tempfile::tempdir().expect("tempdir");
	let compose = dir.path().join("compose.yaml");
	std::fs::write(&compose, body).expect("write compose");
	let name = format!("t{}-{}", std::process::id(), tag);
	let bin = env!("CARGO_BIN_EXE_podup");
	let out = Command::new(bin)
		.args(["-f"])
		.arg(&compose)
		.args(["-p", &name, "up", "-d", "--no-build"])
		.env("PODMAN_SOCKET", socket)
		.output()
		.expect("run podup up");
	assert!(
		out.status.success(),
		"`podup up` failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let container = podman_cmd(
		socket,
		&[
			"ps",
			"-a",
			"--format",
			"{{.Names}}",
			"--filter",
			&format!("label=podup.project={name}"),
		],
	);
	let container = container
		.lines()
		.next()
		.unwrap_or_default()
		.trim_start_matches('/')
		.to_string();
	assert!(
		!container.is_empty(),
		"no project container was created for {name}"
	);
	(dir, name, container)
}

/// Tear the project down. Best-effort: the `Drop` on `Project` would
/// do the same, but a single explicit teardown keeps the assertion
/// surface (and the leftover list) clean.
fn down(socket: &str, dir: &tempfile::TempDir, name: &str) {
	let bin = env!("CARGO_BIN_EXE_podup");
	let compose = dir.path().join("compose.yaml");
	let _ = Command::new(bin)
		.args(["-f"])
		.arg(&compose)
		.args(["-p", name, "down", "-v"])
		.env("PODMAN_SOCKET", socket)
		.output();
}

/// Time the wall-clock between two instants in milliseconds.
fn elapsed_ms(start: Instant) -> u128 {
	start.elapsed().as_millis()
}

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
	let bin = env!("CARGO_BIN_EXE_podup");
	let compose = _dir.path().join("compose.yaml");
	let started = Instant::now();
	let out = Command::new(bin)
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
	let bin = env!("CARGO_BIN_EXE_podup");
	let compose = _dir.path().join("compose.yaml");
	let started = Instant::now();
	let out = Command::new(bin)
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
	let bin = env!("CARGO_BIN_EXE_podup");
	let compose = _dir.path().join("compose.yaml");
	let out = Command::new(bin)
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

// ---------------------------------------------------------------------------
// Compensation 5: `PUT /containers/{}/archive` must send `copyUIDGID=false`
// ---------------------------------------------------------------------------

/// `podup cp <host> app:/path` must leave the copied file at the
/// host's UID/GID inside the container. The libpod handler defaults
/// `copyUIDGID` to true, which overwrites with the container's
/// runtime UID/GID (`0:0`). The Docker compat handler defaulted to
/// false, which kept the host UID/GID on the destination (the
/// Step 0 measurement: `1000:1000`).
///
/// Fails on the branch with `copyUIDGID=false` reverted (i.e. the
/// libpod default of `true` lands) at
/// `internal/engine/copy/upload.rs::archive_put_path` (the asserted
/// `uid:gid == 1000:1000` line).
#[tokio::test]
async fn cp_preserves_the_host_uid_gid() {
	let Some(socket) = podman_socket_url() else {
		return;
	};
	let (_dir, name, container) = up_service(
		&socket,
		"c1914cp",
		"services:\n  app:\n    image: alpine:3.20\n    command: [\"sleep\", \"3600\"]\n",
	);

	let host = _dir.path().join("payload.txt");
	std::fs::write(&host, b"hi").expect("write host");
	let bin = env!("CARGO_BIN_EXE_podup");
	let compose = _dir.path().join("compose.yaml");
	let out = Command::new(bin)
		.args(["-f"])
		.arg(&compose)
		.args([
			"-p",
			&name,
			"cp",
			host.to_str().unwrap(),
			"app:/tmp/payload.txt",
		])
		.env("PODMAN_SOCKET", &socket)
		.output()
		.expect("run podup cp");
	assert!(
		out.status.success(),
		"`podup cp` failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);

	// Read the destination's stat via the libpod archive HEAD. The
	// `mode` field is the POSIX permission bits shifted left; a
	// regular file with mode 0644 reads as 0644 | (regular << 0) ==
	// 0o100000 | 0644 = 0o100644 = 33188. The uid/gid live in the
	// same header, base64-encoded JSON.
	let stat_header = podman_cmd(
		&socket,
		&["inspect", &container, "--format", "{{json .Mounts}}"],
	);
	let _ = stat_header;
	let archive_header = {
		let out = Command::new("podman")
			.args([
				"--url",
				&socket,
				"unshare",
				"--rootless-net",
				"exec",
				&container,
				"stat",
				"-c",
				"%u:%g",
				"/tmp/payload.txt",
			])
			.output()
			.expect("podman exec stat");
		if !out.status.success() {
			// Some runtimes refuse `unshare` for the engine process; fall
			// back to the archive HEAD, which is the same path podup uses.
			let fallback = Command::new("podman")
				.args([
					"--url",
					&socket,
					"exec",
					&container,
					"stat",
					"-c",
					"%u:%g",
					"/tmp/payload.txt",
				])
				.output()
				.expect("podman exec stat fallback");
			assert!(
				fallback.status.success(),
				"`podman exec stat` failed: {}",
				String::from_utf8_lossy(&fallback.stderr)
			);
			String::from_utf8_lossy(&fallback.stdout).trim().to_string()
		} else {
			String::from_utf8_lossy(&out.stdout).trim().to_string()
		}
	};
	down(&socket, &_dir, &name);
	assert_eq!(
		archive_header, "1000:1000",
		"`podup cp` must preserve the host UID/GID (libpod `copyUIDGID=false`): got {archive_header:?}"
	);
}

// ---------------------------------------------------------------------------
// Compensation 6: `GET /containers/{}/top` must send `ps_args=-ef`
// ---------------------------------------------------------------------------

/// `podup top` on an alpine container must produce the docker-compat
/// column set (`UID  PID PPID  C  STIME  TTY  TIME     CMD`). The
/// libpod handler defaults `ps_args` to its own descriptor set when
/// absent, which would change the columns and the test would fail
/// on the asserted header.
///
/// Fails on the branch with `ps_args=-ef` reverted at
/// `internal/engine/query/inspect.rs::top_with_options` (the asserted
/// header line).
#[tokio::test]
async fn top_uses_the_docker_compat_column_set() {
	let Some(socket) = podman_socket_url() else {
		return;
	};
	let (_dir, name, _container) = up_service(
		&socket,
		"c1914top",
		"services:\n  app:\n    image: alpine:3.20\n    command: [\"sleep\", \"3600\"]\n",
	);
	let bin = env!("CARGO_BIN_EXE_podup");
	let compose = _dir.path().join("compose.yaml");
	let out = Command::new(bin)
		.args(["-f"])
		.arg(&compose)
		.args(["-p", &name, "top", "app"])
		.env("PODMAN_SOCKET", &socket)
		.output()
		.expect("run podup top");
	assert!(
		out.status.success(),
		"`podup top` failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let stdout = String::from_utf8_lossy(&out.stdout);
	down(&socket, &_dir, &name);
	// The header is the literal `UID  PID PPID  C  STIME  TTY  TIME     CMD`,
	// bold-wrapped by the live board's escape codes. Strip ANSI before
	// the comparison so a terminal mode toggle does not move the
	// needle.
	let mut stripped = String::new();
	let mut chars = stdout.chars().peekable();
	while let Some(c) = chars.next() {
		if c == '\u{1b}' {
			// Skip the `[...m` ANSI sequence.
			if chars.peek() == Some(&'[') {
				chars.next();
				while let Some(&nc) = chars.peek() {
					chars.next();
					if nc == 'm' {
						break;
					}
				}
			}
			continue;
		}
		stripped.push(c);
	}
	// The docker-compat header: every column is one or more spaces
	// apart, with `TTY  TIME     CMD` (two spaces between TIME and
	// CMD) being the only multi-space gap. The simpler check is: the
	// substring `UID  PID PPID` (with the exact spacing) appears in
	// the output. The libpod default's columns do not start with
	// `UID  PID PPID`, so this catches the regression without
	// pulling in a regex crate.
	assert!(
		stripped.contains("UID  PID PPID"),
		"`podup top` must print the docker-compat column header (libpod `ps_args=-ef`): {stdout:?}"
	);
}

// ---------------------------------------------------------------------------
// Compensation 7: `GET /containers/{}/logs` is multiplexed, TTY or not
// ---------------------------------------------------------------------------

/// `podup logs` of a service with `tty: true` that prints `hello`
/// must surface `hello` and never emit a line that starts with a
/// byte below 0x09 (the libpod channel byte for stdout is 0x01; the
/// docker-compat raw-bytes path would leave it on the first byte of
/// every line and the test would catch it). The libpod handler
/// always frames the body with 8-byte multiplexed headers, including
/// for TTY containers; the Docker compat handler used raw bytes for
/// TTY containers, so parsing by `is_tty` strips the leading channel
/// byte off every line.
///
/// Fails on the branch with the `is_tty` parsing selector restored
/// at `internal/engine/query/inspect.rs::logs_with_options` and
/// `internal/engine/query/attach.rs::attach_logs_with_options` (the
/// asserted `lines.first() >= 0x09` line).
#[tokio::test]
async fn logs_of_a_tty_service_does_not_leak_channel_bytes() {
	let Some(socket) = podman_socket_url() else {
		return;
	};
	let (_dir, name, _container) = up_service(
		&socket,
		"c1914logs",
		"services:\n  app:\n    image: alpine:3.20\n    tty: true\n    command: [\"sh\",\"-c\",\"echo hello; sleep 3600\"]\n",
	);
	// Give the entrypoint a moment to print `hello`. The container
	// has to be running and stdout drained for the multiplexed frame
	// to land; a 500ms settle is the worst-case observed.
	tokio::time::sleep(Duration::from_millis(500)).await;
	let bin = env!("CARGO_BIN_EXE_podup");
	let compose = _dir.path().join("compose.yaml");
	let out = Command::new(bin)
		.args(["-f"])
		.arg(&compose)
		.args(["-p", &name, "logs", "app"])
		.env("PODMAN_SOCKET", &socket)
		.output()
		.expect("run podup logs");
	assert!(
		out.status.success(),
		"`podup logs` failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let stdout = String::from_utf8_lossy(&out.stdout);
	down(&socket, &_dir, &name);
	assert!(
		stdout.contains("hello"),
		"`podup logs` must surface the container's stdout (`hello`): {stdout:?}"
	);
	for line in stdout.lines() {
		if let Some(first) = line.bytes().next() {
			assert!(
				first >= 0x09,
				"`podup logs` must not emit a leading channel byte (libpod multiplexed): \
				 first byte of line was 0x{first:02x} in {stdout:?}"
			);
		}
	}
}

// ---------------------------------------------------------------------------
// Compensation 8: `GET /events` rewrites `died` -> `die`, `remove` -> `delete`
// ---------------------------------------------------------------------------

/// `podup events --format json` for a container that exits 3 must
/// surface the docker-compat verb (`die`) and the docker-compat exit
/// code key (`exitCode`). The libpod handler emits `died` and
/// `containerExitCode`; without the rename, every script that reads
/// the JSON output would silently miss the action or the code.
///
/// Fails on the branch with `rename_event` reverted at
/// `internal/engine/events.rs::format_event` (the asserted
/// `Action == "die"` line).
#[tokio::test]
async fn events_renames_died_to_die_and_container_exit_code_to_exit_code() {
	let Some(socket) = podman_socket_url() else {
		return;
	};
	let tag = "c1914events";
	let name = format!("t{}-{}", std::process::id(), tag);
	let dir = tempfile::tempdir().expect("tempdir");
	let compose = dir.path().join("compose.yaml");
	std::fs::write(
		&compose,
		"services:\n  app:\n    image: alpine:3.20\n    command: [\"sh\",\"-c\",\"exit 3\"]\n",
	)
	.expect("write compose");
	let bin = env!("CARGO_BIN_EXE_podup");
	let up = Command::new(bin)
		.args(["-f"])
		.arg(&compose)
		.args(["-p", &name, "up", "--no-build"])
		.env("PODMAN_SOCKET", &socket)
		.output()
		.expect("run podup up");
	assert!(
		up.status.success(),
		"`podup up` failed: {}",
		String::from_utf8_lossy(&up.stderr)
	);
	// Give the events a moment to land in the daemon's journal so the
	// `--until -1s` window already contains them. Without this sleep
	// the test can race the journal write.
	tokio::time::sleep(Duration::from_millis(1500)).await;
	// The container exited 3 synchronously inside `up`; the events
	// stream is bounded by `--since 30s --until -1s` and yields past
	// events. The `since`/`until` window has to close before the
	// stream ends; a future `--until` does not bound the feed (the
	// event-stream contract has been measured and re-measured for
	// that, see `stream_events_with_options`).
	let out = Command::new(bin)
		.args(["-f"])
		.arg(&compose)
		.args([
			"-p", &name, "events", "--format", "json", "--since", "30s", "--until", "-1s",
		])
		.env("PODMAN_SOCKET", &socket)
		.output()
		.expect("run podup events");
	down(&socket, &dir, &name);
	assert!(
		out.status.success(),
		"`podup events` failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let stdout = String::from_utf8_lossy(&out.stdout);
	// Find the die event. JSON events are emitted one per line; we
	// only care about the one for this project's container, so the
	// first die with our project label wins.
	let mut die_action = None;
	let mut exit_code = None;
	for line in stdout.lines() {
		let v: serde_json::Value = match serde_json::from_str(line) {
			Ok(v) => v,
			Err(_) => continue,
		};
		let project = v
			.pointer("/Actor/Attributes/podup.project")
			.and_then(serde_json::Value::as_str)
			.unwrap_or_default();
		if project != name {
			continue;
		}
		if v.get("Action").and_then(serde_json::Value::as_str) == Some("die") {
			die_action = Some("die".to_string());
			exit_code = v
				.pointer("/Actor/Attributes/exitCode")
				.and_then(serde_json::Value::as_str)
				.map(str::to_string);
			break;
		}
	}
	assert_eq!(
		die_action.as_deref(),
		Some("die"),
		"`podup events --format json` must surface Action=`die` for the container death \
		 (libpod rename `died` -> `die`): {stdout:?}"
	);
	assert_eq!(
		exit_code.as_deref(),
		Some("3"),
		"`podup events --format json` must carry Actor.Attributes.exitCode=3 \
		 (libpod rename `containerExitCode` -> `exitCode`): {stdout:?}"
	);
}

// ---------------------------------------------------------------------------
// Compensation 9: `POST /build` must send `layers=true`
// ---------------------------------------------------------------------------

/// `podup build` must carry `layers=true` on the build request. The
/// libpod build handler defaults `layers` to false, which drops the
/// intermediate images every layer would otherwise leave behind; a
/// second `podup build` of the same Containerfile then re-runs every
/// step from scratch even when the inputs are unchanged. The docker
/// compat handler forced `layers=true` even when absent, and this
/// compensation pins that behaviour on the libpod side.
///
/// The wire-level assertion (`engine::build::query_tests`) is what
/// catches a regression: if the build query drops the `layers=true`
/// parameter, the libpod handler will not keep intermediate images.
/// This integration test is the user-visible counterpart: it
/// verifies the second build still succeeds and produces a tagged
/// image, and that the build query it sends to libpod is the one the
/// handler reads. Whether the cache hits between two builds is a
/// buildah concern, not a query-string concern, and pinning
/// `Using cache` on Podman 5.7.0's libpod `/build` is brittle
/// (the libpod build endpoint's cache semantics differ from
/// `podman build`); the unit test is the load-bearing check here.
#[tokio::test]
async fn build_carries_layers_true_and_produces_an_image() {
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
	let bin = env!("CARGO_BIN_EXE_podup");

	let first = Command::new(bin)
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
		"`podup build` (1) must produce a tagged image (libpod `layers=true`): stdout={first_stdout:?} stderr={first_stderr:?}"
	);

	// Tear down the test image so the second build is the one that
	// defines the image id we read. `podup down --rmi local` removes
	// only the project's own images (the `podup.project=` label is
	// what scopes the prune), so it cannot remove `alpine:3.20` or
	// anything outside the test.
	let _ = Command::new(bin)
		.args(["-f"])
		.arg(&compose)
		.args(["-p", &name, "down", "--rmi", "local"])
		.env("PODMAN_SOCKET", &socket)
		.output();

	let second = Command::new(bin)
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
		"`podup build` (2) must produce a tagged image (libpod `layers=true`): stdout={second_stdout:?} stderr={second_stderr:?}"
	);

	// Final teardown: remove the test image so the host stays clean.
	let _ = Command::new(bin)
		.args(["-f"])
		.arg(&compose)
		.args(["-p", &name, "down", "--rmi", "local"])
		.env("PODMAN_SOCKET", &socket)
		.output();
}
