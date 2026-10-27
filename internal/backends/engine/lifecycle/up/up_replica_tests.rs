//! A `service_healthy` dependency waits for every replica of the dependency.
//!
//! The wait on a dependency used to cover the dependency's first replica only,
//! so a dependency whose second replica had already crashed was reported
//! satisfied and the dependent started on top of a broken dependency.

use std::collections::HashMap;

use crate::engine::fake_podman::{self, FakePodman};
use crate::engine::Engine;

/// A container that is running and reports itself healthy.
const HEALTHY: &str = r#"{"State":{"Status":"running","Health":{"Status":"healthy"}}}"#;

/// A container that exited with a non-zero code: it can never become healthy,
/// so the wait must fail rather than report the dependency satisfied.
const CRASHED: &str = r#"{"State":{"Status":"exited","ExitCode":7}}"#;

/// The compose file under test: `web` waits for `db` to be healthy, and `db`
/// runs two replicas.
const FILE: &str = "\
services:
  db:
    image: x
    healthcheck:
      test: [\"CMD\", \"true\"]
    deploy:
      replicas: 2
  web:
    image: x
    depends_on:
      db:
        condition: service_healthy
";

/// A fake that answers the container inspect of each named container with the
/// given fixture, and 404s for any other.
fn host(inspects: &[(&str, &str)]) -> FakePodman {
	let map: HashMap<String, String> = inspects
		.iter()
		.map(|(name, json)| (name.to_string(), json.to_string()))
		.collect();
	fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/healthcheck") {
			return (200, r#"{"Status":"healthy"}"#.to_string());
		}
		if method == "GET" && target.contains("/containers/json") {
			return (200, "[]".to_string());
		}
		if method == "GET" && target.contains("/containers/") && target.ends_with("/json") {
			let name = target
				.split("/containers/")
				.nth(1)
				.and_then(|rest| rest.split('/').next())
				.unwrap_or_default();
			return match map.get(name) {
				Some(json) => (200, json.clone()),
				None => (404, r#"{"message":"no such container"}"#.to_string()),
			};
		}
		if method == "GET" && target.contains("/images/") {
			return (200, r#"{"Id":"aaaa"}"#.to_string());
		}
		if method == "POST" {
			return (200, r#"{"Id":"cafe"}"#.to_string());
		}
		if method == "DELETE" {
			return (200, String::new());
		}
		(404, r#"{"message":"not found"}"#.to_string())
	})
}

async fn up(
	fake: &FakePodman,
	file: &crate::compose::types::ComposeFile,
) -> crate::error::Result<()> {
	Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir())
		.up_with_options(file, false, &[], &[], false, false, false, false)
		.await
}

/// The second replica of the dependency crashed. The dependency is not
/// satisfied, so the `up` that waits for it fails instead of starting the
/// dependent on top of a broken dependency.
#[tokio::test]
async fn a_dependency_whose_second_replica_crashed_fails_the_up() {
	let fake = host(&[("proj-db-1", HEALTHY), ("proj-db-2", CRASHED)]);
	let file = crate::parse_str(FILE).expect("the fixture parses");

	let error = up(&fake, &file)
		.await
		.expect_err("a crashed replica must fail the dependency wait");

	assert!(
		matches!(
			&error,
			crate::error::ComposeError::WaitServiceExited { container, code }
				if container == "proj-db-2" && *code == 7
		),
		"the crashed replica must be the reported cause, got {error:?}"
	);
}

/// The control on the test above: with every replica of the dependency
/// healthy, the `up` succeeds.
#[tokio::test]
async fn a_dependency_whose_replicas_are_all_healthy_starts_the_dependent() {
	let fake = host(&[("proj-db-1", HEALTHY), ("proj-db-2", HEALTHY)]);
	let file = crate::parse_str(FILE).expect("the fixture parses");

	up(&fake, &file)
		.await
		.expect("every replica of the dependency is healthy");

	let requests = fake.requests.lock().unwrap().clone();
	assert!(
		requests
			.iter()
			.any(|r| r.contains("/containers/proj-db-2/json")),
		"the second replica is inspected: {requests:?}"
	);
}
