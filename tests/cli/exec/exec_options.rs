//! What `exec` asks the engine to create: the flags reach the exec request,
//! and `--index` picks the replica it goes to. The engine is a fake Unix
//! socket, so nothing here touches Podman. Every run detaches (`-d`), which
//! keeps the session off the hijacked stream a fake cannot serve.

#![cfg(unix)]

use std::fs;

#[path = "../../hostile/support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

const COMPOSE: &str = "\
services:
  web:
    image: web-image
    deploy:
      replicas: 2
";

fn replica(name: &str) -> String {
	format!(
		r#"{{"Id":"{id}","Names":["{name}"],"Image":"web-image","ImageID":"","State":"running","Status":"","Labels":{{"podup.project":"proj","podup.service":"web"}},"Created":"2026-08-01T00:58:45Z","StartedAt":1,"Ports":[]}}"#,
		id = name.len().to_string().repeat(64),
	)
}

fn two_replicas(line: &str, _: &str) -> Option<Reply> {
	if line.starts_with("GET") && line.contains("/containers/json") {
		return Some(Reply {
			status: 200,
			body: format!("[{},{}]", replica("proj-web-1"), replica("proj-web-2")),
		});
	}
	if line.starts_with("POST") && line.ends_with("/exec HTTP/1.1") {
		return Some(Reply {
			status: 200,
			body: r#"{"Id":"exec-1"}"#.to_string(),
		});
	}
	if line.contains("/exec/") && line.contains("/start") {
		return Some(Reply {
			status: 200,
			body: String::new(),
		});
	}
	None
}

fn engine() -> FakeEngine {
	let engine = FakeEngine::start(two_replicas);
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	engine
}

fn exec(engine: &FakeEngine, args: &[&str]) -> (Option<i32>, String) {
	let mut full = vec!["-f", "compose.yaml", "-p", "proj", "exec", "-d"];
	full.extend_from_slice(args);
	full.extend(["web", "true"]);
	engine.run_with_status(None, &[], &full)
}

/// The request that creates the exec session: its target line and its body.
fn create(engine: &FakeEngine) -> (String, String) {
	let sent = engine.sent();
	let request = sent
		.split("\nPOST ")
		.find(|r| {
			r.lines()
				.next()
				.is_some_and(|l| l.contains("/exec HTTP/1.1"))
		})
		.map(|r| format!("POST {r}"))
		.unwrap_or_else(|| panic!("no exec was created:\n{sent}"));
	let line = request.lines().next().unwrap().to_string();
	let body = request.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
	(line, body)
}

fn json(body: &str) -> serde_json::Value {
	serde_json::from_str(body.trim())
		.unwrap_or_else(|e| panic!("exec body is not JSON ({e}): {body}"))
}

#[test]
fn without_flags_the_exec_asks_for_nothing_extra() {
	let engine = engine();
	let (code, output) = exec(&engine, &[]);
	assert_eq!(code, Some(0), "{output}");
	let (line, body) = create(&engine);
	assert!(line.contains("/containers/proj-web-1/exec"), "{line}");
	let body = json(&body);
	assert_eq!(body["Cmd"], serde_json::json!(["true"]));
	for key in ["Privileged", "User", "WorkingDir", "Env"] {
		assert!(body.get(key).is_none(), "{key} must be absent: {body}");
	}
}

#[test]
fn privileged_reaches_the_exec_request() {
	let engine = engine();
	let (code, output) = exec(&engine, &["--privileged"]);
	assert_eq!(code, Some(0), "{output}");
	assert_eq!(json(&create(&engine).1)["Privileged"], true);
}

#[test]
fn user_reaches_the_exec_request() {
	let engine = engine();
	let (code, output) = exec(&engine, &["-u", "1000:1000"]);
	assert_eq!(code, Some(0), "{output}");
	assert_eq!(json(&create(&engine).1)["User"], "1000:1000");
}

#[test]
fn workdir_reaches_the_exec_request() {
	let engine = engine();
	let (code, output) = exec(&engine, &["-w", "/srv/app"]);
	assert_eq!(code, Some(0), "{output}");
	assert_eq!(json(&create(&engine).1)["WorkingDir"], "/srv/app");
}

#[test]
fn every_env_flag_reaches_the_exec_request() {
	let engine = engine();
	let (code, output) = exec(&engine, &["-e", "A=1", "--env", "B=two words"]);
	assert_eq!(code, Some(0), "{output}");
	assert_eq!(
		json(&create(&engine).1)["Env"],
		serde_json::json!(["A=1", "B=two words"])
	);
}

#[test]
fn index_picks_the_replica_the_exec_goes_to() {
	let engine = engine();
	let (code, output) = exec(&engine, &["--index", "2"]);
	assert_eq!(code, Some(0), "{output}");
	assert!(
		create(&engine).0.contains("/containers/proj-web-2/exec"),
		"{}",
		create(&engine).0
	);
}

#[test]
fn an_index_beyond_the_replicas_is_refused_before_anything_is_created() {
	let engine = engine();
	let (code, output) = exec(&engine, &["--index", "3"]);
	assert!(matches!(code, Some(c) if c != 0), "{output}");
	assert!(
		engine
			.sent()
			.lines()
			.all(|l| !l.ends_with("/exec HTTP/1.1")),
		"no exec may be created for a replica that does not exist:\n{}",
		engine.sent()
	);
}

#[test]
fn detach_starts_the_session_detached_and_does_not_wait_for_a_status() {
	let engine = engine();
	let (code, output) = exec(&engine, &[]);
	assert_eq!(code, Some(0), "{output}");
	let sent = engine.sent();
	assert!(
		sent.contains(r#""Detach":true"#),
		"the session starts detached:\n{sent}"
	);
	assert!(
		!sent.contains("/exec/exec-1/json"),
		"a detached exec has no status to read:\n{sent}"
	);
}
