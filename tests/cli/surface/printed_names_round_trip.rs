//! Every identifier that `ps`, `ls` or `images` print is accepted back by the
//! commands that take a target. The engine is a fake Unix socket that lists one
//! running container, so nothing here touches Podman.

#![cfg(unix)]

use std::fs;

#[path = "../../hostile/support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

const COMPOSE: &str = "\
name: demo
services:
  web:
    image: web-image
  api-v2:
    image: api-image
    container_name: custom-api
";

fn container(name: &str, service: &str) -> String {
	format!(
		r#"{{"Id":"{id}","Names":["{name}"],"Image":"web-image","ImageID":"","State":"running","Status":"","Labels":{{"podup.project":"demo","podup.service":"{service}","podup.config-files":"compose.yaml"}},"Created":"2026-08-01T00:58:45Z","StartedAt":1,"Ports":[]}}"#,
		id = "a".repeat(64),
	)
}

fn lists_two_containers(line: &str, _: &str) -> Option<Reply> {
	let is_list = line.starts_with("GET") && line.contains("/containers/json");
	is_list.then(|| Reply {
		status: 200,
		body: format!(
			"[{},{}]",
			container("demo-web-1", "web"),
			container("custom-api", "api-v2")
		),
	})
}

fn engine() -> FakeEngine {
	let engine = FakeEngine::start(lists_two_containers);
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	engine
}

fn lines(text: &str) -> Vec<String> {
	text.lines()
		.map(str::trim)
		.filter(|l| !l.is_empty())
		.map(str::to_string)
		.collect()
}

/// The words in the column called `header` of a printed table.
fn column(table: &str, header: &str) -> Vec<String> {
	let rows = lines(table);
	let index = rows[0]
		.split_whitespace()
		.position(|w| w == header)
		.unwrap_or_else(|| panic!("no {header} column in:\n{table}"));
	rows[1..]
		.iter()
		.filter_map(|r| r.split_whitespace().nth(index).map(str::to_string))
		.collect()
}

fn run(engine: &FakeEngine, args: &[&str]) -> (Option<i32>, String) {
	let mut full = vec!["-f", "compose.yaml"];
	full.extend_from_slice(args);
	engine.run_with_status(None, &[], &full)
}

/// Stdout only: `run_with_status` merges both streams, and a note on stderr
/// is not an identifier.
fn printed(engine: &FakeEngine, args: &[&str]) -> String {
	let socket = engine.dir().join("podman.sock");
	let out = std::process::Command::new(env!("CARGO_BIN_EXE_podup"))
		.current_dir(engine.dir())
		.arg("--socket")
		.arg(socket)
		.args(["-f", "compose.yaml"])
		.args(args)
		.env("NO_COLOR", "1")
		.env_remove("RUST_LOG")
		.stdin(std::process::Stdio::null())
		.output()
		.expect("run podup");
	assert!(out.status.success(), "{args:?}: {out:?}");
	String::from_utf8(out.stdout).unwrap()
}

/// The service names the three listings print, by each way they print them.
fn printed_services(engine: &FakeEngine) -> Vec<(String, String)> {
	let mut names = Vec::new();
	let mut add = |source: &str, found: Vec<String>| {
		assert!(!found.is_empty(), "{source} printed no service name");
		names.extend(found.into_iter().map(|n| (source.to_string(), n)));
	};
	add("ps", column(&printed(engine, &["ps"]), "SERVICE"));
	add(
		"ps --services",
		lines(&printed(engine, &["ps", "--services"])),
	);
	add(
		"ps --all-services",
		lines(&printed(engine, &["ps", "--all-services"])),
	);
	add("images", column(&printed(engine, &["images"]), "SERVICE"));
	names
}

#[test]
fn every_service_name_the_listings_print_is_accepted_by_the_commands_that_take_one() {
	let engine = engine();
	let printed = printed_services(&engine);
	// Both services must be reachable through a listing, or the loop below
	// proves nothing about the one that is missing.
	for expected in ["web", "api-v2"] {
		assert!(
			printed.iter().any(|(_, n)| n == expected),
			"no listing printed {expected}: {printed:?}"
		);
	}
	for (source, name) in &printed {
		for args in [
			vec!["logs", name.as_str()],
			vec!["exec", "-T", name.as_str(), "true"],
			vec!["restart", name.as_str()],
			vec!["stop", name.as_str()],
		] {
			let (code, output) = run(&engine, &args);
			assert!(
				!output.contains("not found"),
				"`{}` rejected a name that {source} printed ({code:?}):\n{output}",
				args.join(" ")
			);
		}
	}
	// The commands did reach the containers behind those names.
	let sent = engine.sent();
	for container in ["demo-web-1", "custom-api"] {
		assert!(
			sent.contains(&format!("/containers/{container}/")),
			"no command reached {container}:\n{sent}"
		);
	}
}

#[test]
fn every_project_name_ls_prints_is_accepted_as_the_project_to_act_on() {
	let engine = engine();
	let projects = lines(&printed(&engine, &["ls", "--quiet"]));
	assert_eq!(projects, ["demo"], "ls printed the project name");
	for project in &projects {
		for command in ["ps", "stop", "restart"] {
			let (code, output) = run(&engine, &["-p", project, command]);
			assert_eq!(code, Some(0), "`-p {project} {command}`:\n{output}");
		}
	}
}
