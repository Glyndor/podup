//! Auditing, exporting and launching jobs from the command line.
//!
//! `audit` checks a job like a service, `generate quadlet` says jobs are not
//! exported, `run` starts a job and refuses one with `manual: false`, and `up`
//! never starts one. The engine side is a fake Unix socket, so nothing here
//! touches Podman.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

// The fake engine is a Unix domain socket server.
#[cfg(unix)]
#[path = "../../hostile/support/fake_engine.rs"]
mod fake_engine;

/// A socket that cannot be connected to. Every command these tests run
/// names it, so none of them can reach a real Podman.
const NO_ENGINE: &str = "unix:///nonexistent/x.sock";

fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_podup")
}

fn compose(contents: &str) -> (TempDir, PathBuf) {
	let dir = tempfile::tempdir().expect("create temp dir");
	let path = dir.path().join("compose.yaml");
	fs::write(&path, contents).expect("write compose file");
	(dir, path)
}

fn podup(file: &Path, args: &[&str]) -> Output {
	Command::new(bin())
		.args(["--socket", NO_ENGINE])
		.args(["-f", file.to_str().unwrap()])
		.args(args)
		.env("NO_COLOR", "1")
		.env_remove("RUST_LOG")
		.output()
		.expect("run podup")
}

fn stderr(out: &Output) -> String {
	String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
	String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn audit_checks_a_job_like_a_service() {
	let yaml = "services:\n  web:\n    image: nginx\njobs:\n  risky:\n    image: alpine\n    privileged: true\n    triggers: {manual: true}\n";
	let (_dir, file) = compose(yaml);
	let out = podup(&file, &["audit", "--format", "json"]);
	let text = stdout(&out);
	assert!(
		text.contains("risky"),
		"the job is audited: {text}\n{}",
		stderr(&out)
	);
	assert!(text.contains("privileged"), "{text}");
}

#[test]
fn generate_quadlet_exports_a_scheduled_job_and_says_what_it_does_not() {
	let yaml = "services:\n  web:\n    image: nginx\njobs:\n  nightly:\n    image: alpine\n    triggers:\n      schedule: ['0 1 * * *']\n  manual_only:\n    image: alpine\n    triggers: {manual: true}\n";
	let (_dir, file) = compose(yaml);
	let out = podup(&file, &["generate", "quadlet"]);
	assert!(out.status.success(), "{}", stderr(&out));
	let text = stdout(&out);
	assert!(text.contains("web.container"), "{text}");
	assert!(text.contains("-nightly.container"), "{text}");
	assert!(text.contains("-nightly-1.timer"), "{text}");
	assert!(text.contains("OnCalendar=*-*-* 01:00:00"), "{text}");
	assert!(
		!text.contains("manual_only"),
		"no unit for a manual job: {text}"
	);
	let err = stderr(&out);
	assert!(
		err.contains("1 job(s) have no schedule and were not exported (manual_only)"),
		"{err}"
	);
	assert!(err.contains("1 .timer unit(s) were written"), "{err}");
}

#[cfg(unix)]
mod with_an_engine {
	use super::*;

	use crate::fake_engine::{FakeEngine, Reply};

	const COMPOSE: &str = "\
services:
  web:
    image: web-image
  db:
    image: db-image
jobs:
  nightly:
    image: job-image
    depends_on: [db]
    triggers:
      manual: true
      schedule: ['0 1 * * *', '0 2 * * *']
  closed:
    image: closed-image
    triggers:
      manual: false
  chain:
    image: chain-image
    depends_on: [closed]
    triggers: {manual: true}
";

	fn accepts(_: &str, _: &str) -> Option<Reply> {
		None
	}

	fn engine() -> FakeEngine {
		let engine = FakeEngine::start(accepts);
		fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
		engine
	}

	#[test]
	fn up_starts_the_services_and_never_a_job() {
		let engine = engine();
		let output = engine.run(None, &[], &["-f", "compose.yaml", "up", "-d"]);
		let sent = engine.sent();
		assert!(
			sent.contains("web-image"),
			"the service was created: {output}"
		);
		assert!(
			!sent.contains("job-image") && !sent.contains("closed-image"),
			"no job container may be created by `up`:\n{sent}"
		);
		assert!(!sent.contains("nightly"), "{sent}");
		assert_eq!(
			output.matches("job 'nightly'").count(),
			1,
			"`up` reports the scheduled job once: {output}"
		);
	}

	#[test]
	fn run_starts_the_job_and_what_it_depends_on() {
		let engine = engine();
		let output = engine.run(None, &[], &["-f", "compose.yaml", "run", "-d", "nightly"]);
		let sent = engine.sent();
		assert!(
			sent.contains("job-image"),
			"the job container was created: {output}\n{sent}"
		);
		assert!(
			sent.contains("db-image"),
			"its service dependency was started: {sent}"
		);
		assert!(
			!sent.contains("web-image"),
			"an unrelated service is left alone: {sent}"
		);
		assert!(!sent.contains("closed-image"), "{sent}");
		assert_eq!(
			output.matches("job 'nightly'").count(),
			1,
			"`run` reports the schedule once: {output}"
		);
	}

	#[test]
	fn run_refuses_manual_false_and_creates_nothing() {
		let engine = engine();
		let output = engine.run(None, &[], &["-f", "compose.yaml", "run", "-d", "closed"]);
		assert!(
			output.contains("job error: job 'closed' sets `triggers.manual` to false"),
			"{output}"
		);
		assert!(!engine.sent().contains("closed-image"), "{}", engine.sent());
		assert!(
			!engine.sent().contains("containers/create"),
			"{}",
			engine.sent()
		);
	}

	#[test]
	fn run_refuses_a_job_that_would_start_a_closed_one() {
		let engine = engine();
		let output = engine.run(None, &[], &["-f", "compose.yaml", "run", "-d", "chain"]);
		assert!(
			output.contains(
				"job 'chain' depends on job 'closed', which sets `triggers.manual` to false"
			),
			"{output}"
		);
		let sent = engine.sent();
		assert!(
			!sent.contains("chain-image") && !sent.contains("closed-image"),
			"{sent}"
		);
		assert!(
			!sent.lines().any(|l| l.starts_with("POST ")),
			"no container was created: {sent}"
		);
	}

	#[test]
	fn run_of_an_unknown_name_still_says_service_not_found() {
		let engine = engine();
		let output = engine.run(None, &[], &["-f", "compose.yaml", "run", "-d", "ghost"]);
		assert!(output.contains("service 'ghost' not found"), "{output}");
	}
}
