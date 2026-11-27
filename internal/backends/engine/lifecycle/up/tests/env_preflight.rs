//! A service whose environment cannot be read fails `up` before a network, a
//! volume, a secret, a pod or a container is created.

use crate::compose::normalize_default_network;
use crate::engine::fake_podman::{self, FakePodman};
use crate::engine::Engine;
use crate::error::ComposeError;

/// A project in a pod, with a volume and a secret, whose `web` service reads
/// the env file `web.env` of the project directory.
const PROJECT: &str = "x-podman-pod: true
volumes:
  data:
secrets:
  token:
    content: x
services:
  web:
    image: alpine
    secrets: [token]
    volumes: ['data:/data']
";

/// Run `up` on `yaml` in `dir`, for `targets`, and return its error and the
/// requests that would have changed something.
async fn up(
	dir: &std::path::Path,
	yaml: &str,
	targets: &[&str],
) -> (Option<ComposeError>, Vec<String>) {
	let fake: FakePodman = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, "[]".to_string())
		} else if method == "GET" {
			(404, r#"{"message":"no such object"}"#.to_string())
		} else {
			(200, "{}".to_string())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "probe".into(), dir.to_path_buf());
	let mut file = crate::parse_str(yaml).expect("fixture must parse");
	normalize_default_network(&mut file);
	let targets: Vec<String> = targets.iter().map(|t| t.to_string()).collect();
	let result = engine
		.up_with_options(&file, false, &[], &targets, false, false, false, false)
		.await;
	let changes = fake
		.requests
		.lock()
		.unwrap()
		.iter()
		.filter(|r| !r.starts_with("GET"))
		.cloned()
		.collect();
	(result.err(), changes)
}

fn dir_with(files: &[(&str, &str)]) -> tempfile::TempDir {
	let dir = tempfile::tempdir().unwrap();
	for (name, text) in files {
		std::fs::write(dir.path().join(name), text).unwrap();
	}
	dir
}

fn web_reads(env_file: &str) -> String {
	format!("{PROJECT}    env_file: [{env_file}]\n")
}

#[tokio::test]
async fn a_missing_env_file_fails_up_before_anything_is_created() {
	let dir = dir_with(&[]);
	let (err, changes) = up(dir.path(), &web_reads("web.env"), &[]).await;
	assert!(
		matches!(err, Some(ComposeError::FileNotFound(ref path)) if path.ends_with("web.env")),
		"got {err:?}"
	);
	assert!(changes.is_empty(), "{changes:?}");
}

#[tokio::test]
async fn an_env_file_that_does_not_parse_fails_up_before_anything_is_created() {
	let dir = dir_with(&[("web.env", "A=\"unterminated\n")]);
	let (err, changes) = up(dir.path(), &web_reads("web.env"), &[]).await;
	assert!(matches!(err, Some(ComposeError::EnvFile(_))), "got {err:?}");
	assert!(changes.is_empty(), "{changes:?}");
}

#[tokio::test]
async fn the_broken_file_of_a_later_service_creates_nothing_for_the_earlier_one() {
	let dir = dir_with(&[("bad.env", "A=\"unterminated\n")]);
	let yaml = "services:
  first:
    image: alpine
  second:
    image: alpine
    env_file: [bad.env]
    depends_on: [first]
";
	let (err, changes) = up(dir.path(), yaml, &[]).await;
	assert!(matches!(err, Some(ComposeError::EnvFile(_))), "got {err:?}");
	assert!(changes.is_empty(), "{changes:?}");
}

#[tokio::test]
async fn an_optional_env_file_that_is_missing_does_not_stop_up() {
	let dir = dir_with(&[]);
	let yaml = format!("{PROJECT}    env_file: [{{path: web.env, required: false}}]\n");
	let (_, changes) = up(dir.path(), &yaml, &[]).await;
	assert!(
		changes.iter().any(|r| r.contains("/networks/create")),
		"up did not go on: {changes:?}"
	);
}

/// A dependency may be what writes the file, so a missing one is read again
/// after the dependency ran.
#[tokio::test]
async fn a_missing_file_of_a_dependent_service_is_left_for_its_turn() {
	let dir = dir_with(&[]);
	let yaml = "services:
  generate:
    image: alpine
  app:
    image: alpine
    env_file: [late.env]
    depends_on: [generate]
";
	let (err, changes) = up(dir.path(), yaml, &[]).await;
	assert!(
		changes.iter().any(|r| r.contains("/containers/create")),
		"the dependency was not started: {changes:?}"
	);
	assert!(
		matches!(err, Some(ComposeError::FileNotFound(_))),
		"got {err:?}"
	);
}

#[tokio::test]
async fn a_service_this_up_does_not_start_is_not_read() {
	let dir = dir_with(&[]);
	let yaml = "services:
  web:
    image: alpine
  other:
    image: alpine
    env_file: [absent.env]
";
	let (err, changes) = up(dir.path(), yaml, &["web"]).await;
	assert!(
		!matches!(err, Some(ComposeError::FileNotFound(_))),
		"got {err:?}"
	);
	assert!(
		changes.iter().any(|r| r.contains("/networks/create")),
		"up did not go on: {changes:?}"
	);
}
