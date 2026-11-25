//! The exit status of a foreground `up`: `--exit-code-from` makes the named
//! container's code podup's own, and `--abort-on-container-exit` stops the
//! project when the first container ends. The engine is a fake Unix socket in
//! which `web` never finishes on its own and `db` exits with a failure, so the
//! first container to end is always `db`.

#![cfg(unix)]

use std::fs;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

#[path = "../../hostile/support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

const COMPOSE: &str = "\
services:
  web:
    image: web-image
  db:
    image: db-image
";

/// How long a run may take before it is taken for hung and killed. A fixed
/// bound on the whole command, not a wait for something to happen.
const DEADLINE: Duration = Duration::from_secs(60);

fn running(name: &str, service: &str) -> String {
	format!(
		r#"{{"Id":"{id}","Names":["{name}"],"Image":"{service}-image","ImageID":"","State":"running","Status":"","Labels":{{"podup.project":"proj","podup.service":"{service}"}},"Created":"2026-08-01T00:58:45Z","StartedAt":1,"Ports":[]}}"#,
		id = name.len().to_string().repeat(64),
	)
}

/// `web` keeps running (its log stream never ends and its status is 7), `db`
/// ends at once with status 3.
fn web_runs_db_fails(line: &str, _: &str) -> Option<Reply> {
	if line.starts_with("GET") && line.contains("/containers/json") {
		return Some(Reply {
			status: 200,
			body: format!(
				"[{},{}]",
				running("proj-web-1", "web"),
				running("proj-db-1", "db")
			),
		});
	}
	if line.contains("proj-web-1/logs") {
		// Never answered: the stream of a container that keeps running.
		loop {
			std::thread::park();
		}
	}
	if line.contains("/logs") {
		return Some(Reply {
			status: 200,
			body: "\u{1}\0\0\0\0\0\0\u{6}hello\n".into(),
		});
	}
	if line.contains("proj-db-1/wait") {
		return Some(Reply {
			status: 200,
			body: "3".into(),
		});
	}
	if line.contains("proj-web-1/wait") {
		return Some(Reply {
			status: 200,
			body: "7".into(),
		});
	}
	None
}

fn engine() -> FakeEngine {
	let engine = FakeEngine::start_concurrent(web_runs_db_fails);
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	engine
}

/// Run `podup -p proj -f compose.yaml up <extra>` and give back its exit code
/// and everything it printed. A run that does not end within [`DEADLINE`] is
/// killed and reported as a failure of the test.
fn up(engine: &FakeEngine, extra: &[&str]) -> (Option<i32>, String) {
	let child = Command::new(env!("CARGO_BIN_EXE_podup"))
		.current_dir(engine.dir())
		.arg("--socket")
		.arg(engine.dir().join("podman.sock"))
		.args(["-p", "proj", "-f", "compose.yaml", "up"])
		.args(extra)
		.env("NO_COLOR", "1")
		.env_remove("RUST_LOG")
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.expect("run podup");
	let pid = child.id();
	let (tx, rx) = mpsc::channel();
	std::thread::spawn(move || {
		let _ = tx.send(child.wait_with_output());
	});
	let Ok(out) = rx.recv_timeout(DEADLINE) else {
		// SAFETY: a plain signal to a child this test started.
		unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
		panic!("podup {extra:?} did not finish: the run was stopped");
	};
	let out = out.expect("podup output");
	let text = format!(
		"{}\n{}",
		String::from_utf8_lossy(&out.stdout),
		String::from_utf8_lossy(&out.stderr)
	);
	(out.status.code(), text)
}

fn stopped(engine: &FakeEngine, container: &str) -> bool {
	engine
		.sent()
		.lines()
		.any(|l| l.starts_with("POST") && l.contains(&format!("{container}/stop")))
}

#[test]
fn abort_on_container_exit_ends_the_run_with_the_code_of_the_first_container_to_exit() {
	let engine = engine();
	let (code, output) = up(&engine, &["--abort-on-container-exit"]);
	assert_eq!(code, Some(3), "{output}");
}

#[test]
fn abort_on_container_exit_stops_the_container_that_is_still_running() {
	let engine = engine();
	let (_, output) = up(&engine, &["--abort-on-container-exit"]);
	assert!(
		stopped(&engine, "proj-web-1"),
		"the project must be stopped when one container ends: {output}\n{}",
		engine.sent()
	);
}

#[test]
fn exit_code_from_a_service_reports_that_services_code_and_not_the_first_to_exit() {
	let engine = engine();
	let (code, output) = up(&engine, &["--exit-code-from", "web"]);
	assert_eq!(
		code,
		Some(7),
		"web's code (7), not db's (3), which is the container that ended first: {output}"
	);
}

#[test]
fn exit_code_from_the_service_that_ended_first_reports_its_code() {
	let engine = engine();
	let (code, output) = up(&engine, &["--exit-code-from", "db"]);
	assert_eq!(code, Some(3), "{output}");
}

#[test]
fn exit_code_from_implies_the_abort_and_stops_the_rest() {
	let engine = engine();
	let (_, output) = up(&engine, &["--exit-code-from", "db"]);
	assert!(
		stopped(&engine, "proj-web-1"),
		"naming the exit source must end the run when it ends: {output}\n{}",
		engine.sent()
	);
}
