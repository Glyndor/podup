//! Every replica of a service is waited on, not just the first one.
//!
//! `up --wait` reduced each service to its first replica: the other replicas
//! were never inspected, so a service whose second replica had already
//! crashed still counted as ready. A service scaled to zero had no container
//! at all, and the wait used to invent a name for it and ask about a
//! container that does not exist.

use std::collections::HashMap;

use crate::engine::fake_podman::{self, FakePodman};
use crate::engine::Engine;

/// A container that is running and reports itself healthy.
const HEALTHY: &str = r#"{"State":{"Status":"running","Health":{"Status":"healthy"}}}"#;

/// A container that exited with a non-zero code: it can never become healthy,
/// so the wait must fail rather than report the service ready.
const CRASHED: &str = r#"{"State":{"Status":"exited","ExitCode":7}}"#;

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
		(404, r#"{"message":"not found"}"#.to_string())
	})
}

/// The requests the fake answered.
fn requests(fake: &FakePodman) -> Vec<String> {
	fake.requests.lock().unwrap().clone()
}

fn engine(fake: &FakePodman) -> Engine {
	Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir())
}

/// The second replica of a three-replica service crashed. The wait covers
/// every replica of the service, so it fails instead of reporting the service
/// ready on the strength of the first one.
#[tokio::test]
async fn the_wait_covers_every_replica_of_the_service() {
	let file =
		crate::parse_str("services:\n  web:\n    image: x\n    deploy:\n      replicas: 3\n")
			.expect("the fixture parses");
	let fake = host(&[
		("proj-web-1", HEALTHY),
		("proj-web-2", CRASHED),
		("proj-web-3", HEALTHY),
	]);

	let error = engine(&fake)
		.wait_services_healthy(&file, &[])
		.await
		.expect_err("a crashed replica must fail the wait");

	assert!(
		matches!(
			&error,
			crate::error::ComposeError::WaitServiceExited { container, code }
				if container == "proj-web-2" && *code == 7
		),
		"the crashed replica must be the reported cause, got {error:?}"
	);
}

/// The control on the test above: with every replica healthy, the wait
/// succeeds.
#[tokio::test]
async fn the_wait_succeeds_when_every_replica_is_healthy() {
	let file =
		crate::parse_str("services:\n  web:\n    image: x\n    deploy:\n      replicas: 3\n")
			.expect("the fixture parses");
	let fake = host(&[
		("proj-web-1", HEALTHY),
		("proj-web-2", HEALTHY),
		("proj-web-3", HEALTHY),
	]);

	engine(&fake)
		.wait_services_healthy(&file, &[])
		.await
		.expect("every replica is healthy");

	let inspected: Vec<String> = requests(&fake)
		.into_iter()
		.filter(|r| r.contains("/containers/proj-web-"))
		.collect();
	assert_eq!(
		inspected.len(),
		3,
		"every replica is inspected: {inspected:?}"
	);
}

/// A service scaled to zero has no container. There is nothing to wait for, so
/// the wait asks about nothing at all instead of inventing a name and asking
/// about a container that does not exist.
#[tokio::test]
async fn a_service_with_no_replicas_is_not_waited_on() {
	let file = crate::parse_str("services:\n  web:\n    image: x\n    scale: 0\n")
		.expect("the fixture parses");
	let fake = host(&[]);

	engine(&fake)
		.wait_services_healthy(&file, &[])
		.await
		.expect("a service with no replicas has nothing to wait for");

	assert!(
		requests(&fake).is_empty(),
		"no container is inspected: {requests:?}",
		requests = requests(&fake)
	);
}
