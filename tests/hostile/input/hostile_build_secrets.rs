//! Build-secret identifiers and special sources must stay out of ordinary context.
#![cfg(unix)]

use std::fs;
use std::process::Command;

#[path = "../support/fake_engine.rs"]
mod fake_engine;
#[path = "../support/host_files.rs"]
mod host_files;

use fake_engine::FakeEngine;
use host_files::{accepts, entries, Running, MARKER, SAFE};

fn compose(name: &str, source: &str) -> String {
	let name = serde_json::to_string(name).unwrap();
	format!("services:\n  web:\n    build:\n      context: ./context\n      secrets: [{name}]\nsecrets:\n  {name}:\n    {source}\n")
}

#[test]
fn hostile_build_secret_names_and_sources_never_leak_or_inject_specs() {
	let mut failures = Vec::new();
	for (name, source, rejected) in [
		("valid", "file: ./secret.txt", false),
		("a,src=ordinary", "file: ./secret.txt", true),
		("a=b", "file: ./secret.txt", true),
		("line\nbreak", "file: ./secret.txt", true),
		("valid", "file: ./pipe", true),
		("valid", "file: ./link", true),
	] {
		let fake = FakeEngine::start(accepts);
		let outside = tempfile::tempdir().unwrap();
		fs::write(outside.path().join("secret"), MARKER).unwrap();
		fs::write(fake.dir().join("secret.txt"), MARKER).unwrap();
		fs::create_dir(fake.dir().join("context")).unwrap();
		fs::write(
			fake.dir().join("context/Dockerfile"),
			"FROM scratch\nCOPY keep /keep\n",
		)
		.unwrap();
		fs::write(fake.dir().join("context/keep"), SAFE).unwrap();
		assert!(Command::new("mkfifo")
			.arg(fake.dir().join("pipe"))
			.status()
			.unwrap()
			.success());
		std::os::unix::fs::symlink(outside.path().join("secret"), fake.dir().join("link")).unwrap();
		fs::write(fake.dir().join("compose.yaml"), compose(name, source)).unwrap();
		let mut child = Running::start(&fake, &["build"]);
		let status = child.wait();
		let text = child.text();
		let requests = fake.requests();
		let builds: Vec<_> = requests
			.iter()
			.filter(|r| {
				String::from_utf8_lossy(r)
					.lines()
					.next()
					.unwrap_or("")
					.contains("/build?")
			})
			.collect();
		if rejected {
			if status.success()
				|| !builds.is_empty()
				|| !(text.contains("secret") || text.contains("refusing to read"))
			{
				failures.push(format!(
					"name={name:?} source={source}: status={status}; builds={}; {text}",
					builds.len()
				));
			}
		} else {
			assert_eq!(builds.len(), 1, "valid control never reached build: {text}");
			let sent = entries(builds[0]);
			assert!(sent
				.iter()
				.any(|(name, bytes)| name == "keep" && bytes == SAFE.as_bytes()));
			let secret_entries: Vec<_> = sent
				.iter()
				.filter(|(_, bytes)| bytes == MARKER.as_bytes())
				.collect();
			if secret_entries.len() != 1 || secret_entries[0].0 != ".podup-build-secret-valid" {
				failures.push(format!(
					"source={source}: secret leaked into ordinary context entries: {:?}",
					secret_entries
						.iter()
						.map(|(name, _)| name)
						.collect::<Vec<_>>()
				));
			}
			let line = String::from_utf8_lossy(builds[0]);
			assert!(
				line.lines()
					.next()
					.unwrap()
					.contains("id%3Dvalid%2Csrc%3D.podup-build-secret-valid"),
				"missing valid secret spec"
			);
		}
		assert_eq!(
			fs::read(outside.path().join("secret")).unwrap(),
			MARKER.as_bytes()
		);
	}
	assert!(failures.is_empty(), "{}", failures.join("\n"));
}
