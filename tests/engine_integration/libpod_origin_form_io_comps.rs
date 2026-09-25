//! #1914 compensations: read endpoints the libpod switch disturbed.
//!
//! Each test pins one of the four read behaviours: the libpod handler
//! defaults a parameter or changes the byte shape that podup
//! interprets, and podup's user-visible behaviour (the docker-compat
//! shape) is the contract these tests are here to keep honest. Every
//! test runs against the same Podman 5.7.0 socket the build/lifecycle
//! unit tests already assume, and skips cleanly when no daemon is
//! reachable.
//!
//! Each test fails with its compensation removed (the unit tests in
//! `engine::lifecycle::libpod_endpoint_query_tests` and
//! `engine::events_tests` pin the wire shape; these tests pin the
//! user-visible behaviour the wire shape exists to keep). The expected
//! failing line, when the compensation is reverted, is the one the
//! comment on the test names.

#[allow(unused_imports)]
use super::*;
use std::process::Command;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Compensation 5: `PUT /containers/{}/archive` must send `copyUIDGID=false`
// ---------------------------------------------------------------------------

/// `podup cp <host> app:/path` must leave the copied file at the
/// host's UID/GID inside the container. The libpod handler defaults
/// `copyUIDGID` to true, which overwrites with the container's
/// runtime UID/GID (`0:0`). The Docker compat handler defaulted to
/// false, which kept the host UID/GID on the destination (as measured with podup 5.10.0 on
/// 2026-09-24: `1000:1000`).
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
	let compose = _dir.path().join("compose.yaml");
	let out = Command::new(bin())
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

	// Read the destination's stat inside the container. Some runtimes
	// refuse `unshare` for the engine process; the `exec` fallback is
	// the same path podup uses.
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
	let compose = _dir.path().join("compose.yaml");
	let out = Command::new(bin())
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

/// `podup logs` of a service with `tty: true` that prints `tty-hello`
/// must surface `tty-hello` and never emit a byte below 0x09 in the
/// output (the libpod channel byte for stdout is 0x01; the docker-compat
/// raw-bytes path would leave it on the first byte of every line and
/// the test catches it on the first byte of the line, on every byte of
/// the whole output, and on the exact payload of the TTY service).
/// The libpod handler always frames the body with 8-byte multiplexed
/// headers, including for TTY containers; the Docker compat handler
/// used raw bytes for TTY containers, so parsing by `is_tty` strips
/// the leading channel byte off every line.
///
/// The compose carries one non-TTY service (`web`) alongside the TTY
/// service (`term`). The non-TTY service has always been parsed as
/// multiplexed; the regression only affected the TTY branch. Running
/// both services in the same invocation pins both branches through
/// one CLI call, the way a real `podup logs` would.
///
/// Fails on the branch with the `is_tty` parsing selector restored at
/// `internal/engine/query/mod.rs::logs_with_options` (the two arms
/// around lines 349 and 450) at the asserted "the term service's line
/// must be exactly `<prefix>tty-hello` (a trailing `\\r` is allowed)"
/// line and at the byte-below-0x09 panic.
#[tokio::test]
async fn logs_of_a_tty_service_does_not_leak_channel_bytes() {
	let Some(socket) = podman_socket_url() else {
		return;
	};
	let (_dir, name, _container) = up_service(
		&socket,
		"c1914logs",
		"services:\n  web:\n    image: alpine:3.20\n    command: [\"sh\",\"-c\",\"echo web-hello; sleep 3600\"]\n  term:\n    image: alpine:3.20\n    tty: true\n    command: [\"sh\",\"-c\",\"echo tty-hello; sleep 3600\"]\n",
	);
	// Give the entrypoints a moment to print their lines. The
	// containers have to be running and stdout drained for the
	// multiplexed frame to land; a 500ms settle is the worst-case
	// observed.
	tokio::time::sleep(Duration::from_millis(500)).await;
	let compose = _dir.path().join("compose.yaml");
	let out = Command::new(bin())
		.args(["-f"])
		.arg(&compose)
		.args(["-p", &name, "logs", "web", "term"])
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
	// Find the term service's line. `podup logs` prefixes every line
	// with `{service}-{replica} | `, so `term-1 | ` is the prefix for
	// the term container's output. The libpod `/logs` endpoint wraps
	// the TTY payload in an 8-byte multiplexed header; parsing by
	// `is_tty` leaves the channel byte (0x01, the libpod stdout tag)
	// as the first byte of the line. The assertion below catches
	// the byte both by exact-prefix match (the line starts with the
	// prefix and nothing else) and by a byte-by-byte scan (no byte
	// below 0x09 except the newline and carriage return the
	// container's `echo` emitted).
	let term_line = stdout
		.lines()
		.find(|line| line.starts_with("term-1 | "))
		.unwrap_or_else(|| {
			panic!("`podup logs` must print the term service's line; output was:\n{stdout:?}")
		});
	let term_payload = &term_line["term-1 | ".len()..];
	assert!(
		term_payload == "tty-hello",
		"the TTY service's line must be exactly `term-1 | tty-hello` \
		 (the trailing `\\r\\n` from the container's `echo` is stripped by `lines()`, \
		 which the assertion ignores); \
		 got {term_line:?}"
	);
	// A second, byte-by-byte check that catches a regression on the
	// non-TTY branch or anywhere else in the output. The libpod
	// channel bytes for stdout and stderr are 0x01 and 0x02; both
	// are below the printable range. The only bytes below 0x09 the
	// output is allowed to carry are the newline (`\n`) that ends
	// every line and the carriage return (`\r`) the container's
	// `echo` emitted.
	for byte in stdout.bytes() {
		if byte < 0x09 && byte != b'\n' && byte != b'\r' {
			panic!(
				"`podup logs` output contains byte 0x{byte:02x} (below 0x09) \
				 outside `\\n`/`\\r`; the libpod `/logs` response is always \
				 multiplexed, so the parser must strip the channel byte. \
				 Output:\n{stdout:?}"
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
	let up = Command::new(bin())
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
	// Poll the event feed until the project's `die` event appears or
	// we run out of attempts. The CI lane (a nested-virt runner with
	// the journald event backend) sometimes takes several seconds for
	// the first event to land; the original 1.5 s sleep was measured
	// on the local 5.7.0 socket and races the journal write. The
	// container exits 3 synchronously inside `up`, so the `die` event
	// exists by the time `up` returns; the poll only papers over the
	// journal-to-HTTP event bridge on the libpod handler.
	//
	// `--since 60s` covers the settle window and any later attempt's
	// start; `--until 0s` bounds the stream (a future `--until` does
	// not, and an unbounded poll would never return the first die
	// event before the next attempt fired; the event-stream contract
	// is pinned at `stream_events_with_options`).
	const ATTEMPTS: usize = 20;
	const INTERVAL: Duration = Duration::from_secs(1);
	let mut last_stdout = String::new();
	let mut last_stderr = String::new();
	let mut die_action = None;
	let mut exit_code = None;
	for attempt in 0..ATTEMPTS {
		let out = Command::new(bin())
			.args(["-f"])
			.arg(&compose)
			.args([
				"-p", &name, "events", "--format", "json", "--since", "60s", "--until", "0s",
			])
			.env("PODMAN_SOCKET", &socket)
			.output()
			.expect("run podup events");
		assert!(
			out.status.success(),
			"`podup events` failed on attempt {attempt}: {}",
			String::from_utf8_lossy(&out.stderr)
		);
		let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
		// JSON events are emitted one per line; we only care about
		// the one for this project's container, so the first die
		// with our project label wins.
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
		last_stdout = stdout;
		last_stderr = String::from_utf8_lossy(&out.stderr).into_owned();
		if die_action.is_some() {
			break;
		}
		if attempt + 1 < ATTEMPTS {
			tokio::time::sleep(INTERVAL).await;
		}
	}
	// When the event never shows up, say what podman itself saw, so a failure on a
	// runner nobody can log into tells podup's filtering apart from an empty journal.
	let diagnosis = if die_action.is_none() {
		let label = format!("label=podup.project={name}");
		let ps = podman_cmd(
			&socket,
			&[
				"ps",
				"-a",
				"--filter",
				&label,
				"--format",
				"{{.Names}} {{.Status}}",
			],
		);
		let raw = podman_cmd(
			&socket,
			&[
				"events",
				"--since",
				"120s",
				"--until",
				"0s",
				"--filter",
				&label,
				"--format",
				"{{.Status}}",
			],
		);
		format!(
			"up stdout={:?} up stderr={:?} last events stderr={last_stderr:?} podman ps={ps:?} podman events={raw:?}",
			String::from_utf8_lossy(&up.stdout),
			String::from_utf8_lossy(&up.stderr),
		)
	} else {
		String::new()
	};
	down(&socket, &dir, &name);
	assert_eq!(
		die_action.as_deref(),
		Some("die"),
		"`podup events --format json` must surface Action=`die` for the container death \
		 (libpod rename `died` -> `die`, polled {ATTEMPTS} times at {INTERVAL:?} intervals): \
		 {last_stdout:?} {diagnosis}"
	);
	assert_eq!(
		exit_code.as_deref(),
		Some("3"),
		"`podup events --format json` must carry Actor.Attributes.exitCode=3 \
		 (libpod rename `containerExitCode` -> `exitCode`, polled {ATTEMPTS} times at \
		 {INTERVAL:?} intervals): {last_stdout:?}"
	);
}
