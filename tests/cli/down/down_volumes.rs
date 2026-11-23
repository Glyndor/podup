//! `down -v` removes the volumes the file declares and nothing else; `down`
//! alone never touches a volume. The engine is a fake Unix socket that
//! answers the inspect of each volume, so nothing here touches Podman.

#![cfg(unix)]

use std::fs;

#[path = "../../hostile/support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

const COMPOSE: &str = "\
services:
  web:
    image: web-image
    volumes:
      - data:/data
      - kept:/kept
      - shared:/shared
volumes:
  data: {}
  kept:
    name: custom-volume-name
  shared:
    external: true
";

/// The inspect of one volume (`/volumes/<name>/json`), not the listing of all
/// of them (`/volumes/json`).
fn is_volume_inspect(line: &str) -> bool {
	line.starts_with("GET") && line.contains("/volumes/") && !line.contains("/volumes/json")
}

/// Whether a request names one volume, whatever it asks of it.
fn names_a_volume(line: &str) -> bool {
	line.contains("/volumes/") && !line.contains("/volumes/json")
}

fn volume(labels: &str) -> Reply {
	Reply {
		status: 200,
		body: format!(r#"{{"Name":"x","Labels":{labels}}}"#),
	}
}

/// Every declared volume exists and carries this project's label.
fn owned(line: &str, _: &str) -> Option<Reply> {
	is_volume_inspect(line).then(|| volume(r#"{"podup.project":"proj"}"#))
}

/// The volumes exist and carry no label: not podup's to remove.
fn unlabelled(line: &str, _: &str) -> Option<Reply> {
	is_volume_inspect(line).then(|| volume("{}"))
}

/// The volumes belong to another project.
fn someone_elses(line: &str, _: &str) -> Option<Reply> {
	is_volume_inspect(line).then(|| volume(r#"{"podup.project":"other"}"#))
}

/// An orphan container of a service the file no longer declares.
fn owned_with_an_orphan(line: &str, request: &str) -> Option<Reply> {
	if line.starts_with("GET") && line.contains("/containers/json") {
		return Some(Reply {
			status: 200,
			body: format!(
				r#"[{{"Id":"{id}","Names":["proj-gone-1"],"Image":"gone-image","ImageID":"","State":"running","Status":"","Labels":{{"podup.project":"proj","podup.service":"gone"}},"Created":"2026-08-01T00:58:45Z","StartedAt":1,"Ports":[]}}]"#,
				id = "b".repeat(64),
			),
		});
	}
	owned(line, request)
}

/// The project's volumes as the listing of a daemon holds them.
fn listed(line: &str, _: &str) -> Option<Reply> {
	(line.starts_with("GET") && line.ends_with("/volumes/json HTTP/1.1")).then(|| Reply {
		status: 200,
		body: r#"[
			{"Name":"proj_data","Labels":{"podup.project":"proj"}},
			{"Name":"proj_unlabelled","Labels":{}},
			{"Name":"other_data","Labels":{"podup.project":"other"}}
		]"#
		.to_string(),
	})
}

fn engine(handler: fake_engine::Handler) -> FakeEngine {
	let engine = FakeEngine::start(handler);
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	engine
}

fn down(engine: &FakeEngine, args: &[&str]) -> (Option<i32>, String) {
	let mut full = vec!["-f", "compose.yaml", "-p", "proj", "down"];
	full.extend_from_slice(args);
	engine.run_with_status(None, &[], &full)
}

fn lines(engine: &FakeEngine) -> Vec<String> {
	engine
		.sent()
		.lines()
		.filter(|l| l.starts_with("GET ") || l.starts_with("POST ") || l.starts_with("DELETE "))
		.map(str::to_string)
		.collect()
}

fn volume_deletes(lines: &[String]) -> Vec<&String> {
	lines
		.iter()
		.filter(|l| l.starts_with("DELETE") && names_a_volume(l))
		.collect()
}

#[test]
fn down_without_volumes_never_touches_a_volume() {
	let engine = engine(owned);
	let (code, output) = down(&engine, &[]);
	assert_eq!(code, Some(0), "{output}");
	let lines = lines(&engine);
	assert!(
		lines.iter().all(|l| !names_a_volume(l)),
		"plain down must leave every volume alone, even to look at it: {lines:?}"
	);
}

#[test]
fn down_volumes_removes_each_declared_volume_under_its_own_name() {
	let engine = engine(owned);
	let (code, output) = down(&engine, &["-v"]);
	assert_eq!(code, Some(0), "{output}");
	let lines = lines(&engine);
	let deletes = volume_deletes(&lines);
	assert_eq!(deletes.len(), 2, "{lines:?}");
	assert!(
		deletes.iter().any(|l| l.contains("/volumes/proj_data ")),
		"the default name is prefixed with the project: {deletes:?}"
	);
	assert!(
		deletes
			.iter()
			.any(|l| l.contains("/volumes/custom-volume-name ")),
		"a name: override is used verbatim: {deletes:?}"
	);
}

#[test]
fn down_volumes_leaves_an_external_volume_alone() {
	let engine = engine(owned);
	let (code, output) = down(&engine, &["--volumes"]);
	assert_eq!(code, Some(0), "{output}");
	let lines = lines(&engine);
	assert!(
		lines.iter().all(|l| !l.contains("shared")),
		"an external volume is not the project's to inspect or remove: {lines:?}"
	);
}

#[test]
fn a_volume_without_the_project_label_is_refused_and_not_removed() {
	let engine = engine(unlabelled);
	let (code, output) = down(&engine, &["-v"]);
	assert!(matches!(code, Some(c) if c != 0), "{output}");
	assert!(
		output.contains("carries no podup.project label"),
		"the refusal names the missing label: {output}"
	);
	assert!(
		volume_deletes(&lines(&engine)).is_empty(),
		"{:?}",
		lines(&engine)
	);
}

#[test]
fn a_volume_labelled_for_another_project_is_refused_and_not_removed() {
	let engine = engine(someone_elses);
	let (code, output) = down(&engine, &["-v"]);
	assert!(matches!(code, Some(c) if c != 0), "{output}");
	assert!(
		output.contains("labelled podup.project=other"),
		"the refusal names the other project: {output}"
	);
	assert!(
		volume_deletes(&lines(&engine)).is_empty(),
		"{:?}",
		lines(&engine)
	);
}

#[test]
fn volumes_and_orphans_together_remove_both() {
	let engine = engine(owned_with_an_orphan);
	let (code, output) = down(&engine, &["-v", "--remove-orphans"]);
	assert_eq!(code, Some(0), "{output}");
	let lines = lines(&engine);
	assert!(
		lines
			.iter()
			.any(|l| l.starts_with("DELETE") && l.contains("proj-gone-1")),
		"the orphan must be removed: {lines:?}"
	);
	assert_eq!(volume_deletes(&lines).len(), 2, "{lines:?}");
}

#[test]
fn orphans_alone_leave_the_volumes_in_place() {
	let engine = engine(owned_with_an_orphan);
	let (code, output) = down(&engine, &["--remove-orphans"]);
	assert_eq!(code, Some(0), "{output}");
	let lines = lines(&engine);
	assert!(
		lines
			.iter()
			.any(|l| l.starts_with("DELETE") && l.contains("proj-gone-1")),
		"the orphan must be removed: {lines:?}"
	);
	assert!(volume_deletes(&lines).is_empty(), "{lines:?}");
}

/// With no compose file the project is found by label, and `-v` removes the
/// volumes that carry it and no others.
#[test]
fn down_by_label_with_volumes_removes_only_the_projects_volumes() {
	let engine = FakeEngine::start(listed);
	let (code, output) = engine.run_with_status(None, &[], &["-p", "proj", "down", "-v"]);
	assert_eq!(code, Some(0), "{output}");
	let lines = lines(&engine);
	let deletes = volume_deletes(&lines);
	assert_eq!(deletes.len(), 1, "{lines:?}");
	assert!(deletes[0].contains("/volumes/proj_data "), "{deletes:?}");
}

#[test]
fn down_by_label_without_volumes_does_not_list_them() {
	let engine = FakeEngine::start(listed);
	let (code, output) = engine.run_with_status(None, &[], &["-p", "proj", "down"]);
	assert_eq!(code, Some(0), "{output}");
	assert!(
		lines(&engine).iter().all(|l| !l.contains("/volumes")),
		"{:?}",
		lines(&engine)
	);
}
