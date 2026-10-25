//! `down` and `down --rmi` reach the containers and images of jobs.
//!
//! A `run` of a job, and a job started for a job that depends on it, leave
//! containers labelled with the job's name. They belong to the project, so
//! tearing the project down removes them. A container of something the file
//! does not declare at all stays an orphan, removed only by `--remove-orphans`.

use crate::engine::fake_podman;
use crate::engine::Engine;

const FILE: &str = "\
services:
  web:
    image: web-image
jobs:
  nightly:
    image: job-image
    triggers: {manual: true}
";

/// A fake that lists three containers and records every `DELETE`.
fn engine() -> (fake_podman::FakePodman, Engine) {
	let containers = r#"[
		{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"}},
		{"Names":["/proj-nightly-run-77"],"Labels":{"podup.service":"nightly"}},
		{"Names":["/proj-ghost-1"],"Labels":{"podup.service":"ghost"}}
	]"#;
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, containers.to_string())
		} else if method == "POST" && target.contains("/stop") {
			(200, String::new())
		} else if method == "DELETE" {
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	(fake, engine)
}

fn deletes(fake: &fake_podman::FakePodman) -> Vec<String> {
	fake.requests
		.lock()
		.unwrap()
		.iter()
		.filter(|r| r.starts_with("DELETE "))
		.cloned()
		.collect()
}

#[tokio::test]
async fn down_removes_the_containers_of_a_job_and_leaves_an_orphan_alone() {
	let (fake, engine) = engine();
	let file = crate::parse_str(FILE).unwrap();

	engine.down(&file).await.unwrap();

	let removed = deletes(&fake);
	assert!(
		removed.iter().any(|r| r.contains("/proj-web-1")),
		"{removed:?}"
	);
	assert!(
		removed.iter().any(|r| r.contains("/proj-nightly-run-77")),
		"the container a run of the job left is removed: {removed:?}"
	);
	assert!(
		!removed.iter().any(|r| r.contains("/proj-ghost-1")),
		"a container of an undeclared name is an orphan, not part of down: {removed:?}"
	);
}

#[tokio::test]
async fn rmi_removes_the_image_of_a_job() {
	let (fake, engine) = engine();
	let file = crate::parse_str(FILE).unwrap();

	engine.remove_service_images(&file, false).await.unwrap();

	let removed = deletes(&fake);
	assert!(
		removed.iter().any(|r| r.contains("images/web-image")),
		"{removed:?}"
	);
	assert!(
		removed.iter().any(|r| r.contains("images/job-image")),
		"{removed:?}"
	);
}
