use super::filter_orphans;
use std::collections::HashSet;

#[cfg(unix)]
use crate::compose::types::{ComposeFile, Service};
#[cfg(unix)]
use crate::engine::fake_podman;
#[cfg(unix)]
use crate::engine::Engine;
#[cfg(unix)]
use crate::error::ComposeError;

#[cfg(unix)]
fn engine_with(client: crate::libpod::Client, project: &str) -> Engine {
	Engine::with_base_dir(client, project.into(), std::env::temp_dir())
}

/// #598: `--remove-orphans` that can't remove every orphan (e.g. an active
/// exec session) must not exit 0 with one silently left behind, but a
/// sibling orphan that removes cleanly must still be reclaimed.
#[cfg(unix)]
#[tokio::test]
async fn remove_orphans_propagates_a_real_failure_after_completing_the_rest() {
	// "web" is still declared in the file (known); the two "ghost" containers
	// are not, so both are orphans.
	let containers = r#"[
		{"Names":["/proj-web-1"]},
		{"Names":["/proj-ghost-1"]},
		{"Names":["/proj-ghost-2"]}
	]"#;
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, containers.to_string())
		} else if method == "DELETE" && target.contains("/proj-ghost-1?force=true") {
			(200, String::new())
		} else if method == "DELETE" && target.contains("/proj-ghost-2?force=true") {
			(500, r#"{"message":"device or resource busy"}"#.to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let mut file = ComposeFile::default();
	file.services.insert("web".into(), Service::default());

	let err = e
		.remove_orphans(&file)
		.await
		.expect_err("a real orphan-removal failure must propagate");
	assert!(
		matches!(err, ComposeError::Podman(ref pe) if pe.is_status(500)),
		"got {err:?}"
	);

	let seen = fake.requests.lock().unwrap();
	assert!(
		seen.iter()
			.any(|r| r.contains("DELETE") && r.contains("/proj-ghost-1?force=true")),
		"expected proj-ghost-1 to still be removed despite proj-ghost-2 failing: {seen:?}"
	);
}

/// An orphan that is already gone (404) stays an idempotent no-op.
#[cfg(unix)]
#[tokio::test]
async fn remove_orphans_tolerates_already_gone() {
	let containers = r#"[{"Names":["/proj-ghost-1"]}]"#;
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, containers.to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");
	let file = ComposeFile::default();

	e.remove_orphans(&file)
		.await
		.expect("an already-gone orphan must still exit 0");
}

#[test]
fn filter_orphans_keeps_only_unknown_names() {
	let known: HashSet<String> = ["web-1".to_string(), "db".to_string()].into();
	let names = vec![
		"web-1".to_string(),
		"db".to_string(),
		"old-cache".to_string(),
	];
	assert_eq!(filter_orphans(names, &known), vec!["old-cache".to_string()]);
}

#[test]
fn filter_orphans_empty_when_all_known() {
	let known: HashSet<String> = ["web".to_string()].into();
	assert!(filter_orphans(vec!["web".to_string()], &known).is_empty());
}

/// A container the project started for a job is not an orphan: the job is
/// declared in the file, only under `jobs:` instead of `services:`.
#[cfg(unix)]
#[tokio::test]
async fn a_job_container_is_not_an_orphan() {
	let containers = r#"[
		{"Names":["/proj-web-1"]},
		{"Names":["/proj-nightly-1"]},
		{"Names":["/proj-ghost-1"]}
	]"#;
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, containers.to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");
	let file = crate::parse_str(
		"services:\n  web:\n    image: x\njobs:\n  nightly:\n    image: x\n    triggers: {manual: true}\n",
	)
	.unwrap();

	let orphans = e.orphan_container_names(&file).await.unwrap();
	assert_eq!(
		orphans,
		["proj-ghost-1"],
		"only the undeclared one is an orphan"
	);
}
