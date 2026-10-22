//! #1914 compensation 8: the docker-compat vocabulary of the event stream.
//! See `libpod_origin_form_io_comps.rs` for the group this one belongs to and
//! for the rest of the compensations.
#[allow(unused_imports)]
use super::*;
use std::process::Command;
use std::time::Duration;

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
/// `internal/engine/events/mod.rs::format_event` (the asserted
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
	// On the CI lane on 2026-09-25 `podman events` itself returned nothing for a
	// container that had exited 3 (Podman 5.8.1 and 6.1.2, journald backend), so
	// there was no event for podup to rewrite. That host cannot exercise this test:
	// say so in the output rather than fail or pass silently. The rewrite itself is
	// pinned by the unit tests in `internal/engine/events/tests.rs`.
	if die_action.is_none() && diagnosis.contains("podman events=\"\"") {
		eprintln!(
			"SKIP events_renames_died_to_die_and_container_exit_code_to_exit_code: \
			 podman's own event log returned nothing for the project on this host, so the \
			 rewrite was not exercised end to end ({diagnosis})"
		);
		return;
	}
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
