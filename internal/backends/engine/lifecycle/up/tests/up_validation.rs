//! How `up` reacts to an invalid service.
//!
//! Pre-validation runs before any network, volume, secret, pod or
//! container is created, so a rejected `pid`/`ipc`/access-string value
//! surfaces as a field-shaped error with nothing created. A
//! per-service check interleaved with creation leaves the project
//! network behind for the case where the first service is fine and a
//! later one is not; this file pins the up-front check (#1867).

#![cfg(unix)]

use crate::compose::normalize_default_network;
use crate::engine::fake_podman::{self, FakePodman};
use crate::engine::Engine;
use crate::error::ComposeError;
use crate::libpod::error::PodmanError;

fn engine_with(client: crate::libpod::Client, project: &str) -> Engine {
	Engine::with_base_dir(client, project.into(), std::env::temp_dir())
}

/// Parse a fixture string and apply the same post-processing the CLI
/// does (`normalize_default_network`) so `create_networks` has the
/// implicit `default` network to materialise, matching the on-disk
/// path.
fn parse_with_default(yaml: &str) -> crate::compose::types::ComposeFile {
	let mut file = crate::parse_str(yaml).expect("fixture must parse");
	normalize_default_network(&mut file);
	file
}

/// Routes every request to a 200 with a body shaped to satisfy the
/// JSON-parsing callers on the `up` path, so the only thing that can
/// fail an `up` is podup's own pre-validation. The body shapes are the
/// smallest that let the call site deserialise: `GET /containers/json`
/// wants an array (the container list at the top of `run_up`),
/// `POST /networks/create` wants an object (the network-create reply
/// parsed as `serde_json::Value`), and every other request gets an
/// empty body that the call site ignores.
fn recording_fake() -> FakePodman {
	fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, "[]".to_string())
		} else if method == "POST" && target.contains("/networks/create") {
			(200, "{}".to_string())
		} else {
			(200, String::new())
		}
	})
}

fn requests(fake: &FakePodman) -> Vec<String> {
	fake.requests.lock().unwrap().clone()
}

fn requests_creating(fake: &FakePodman) -> Vec<String> {
	requests(fake)
		.into_iter()
		.filter(|r| {
			r.contains("/networks/create")
				|| r.contains("/containers/create")
				|| r.contains("/volumes/create")
				|| r.contains("/secrets/create")
				|| r.contains("/pods/create")
		})
		.collect()
}

fn field_error(err: &ComposeError) -> Option<(String, String, String, String)> {
	if let ComposeError::Podman(PodmanError::Field {
		service,
		field,
		value,
		message,
	}) = err
	{
		Some((
			service.clone(),
			field.clone(),
			value.clone(),
			message.clone(),
		))
	} else {
		None
	}
}

/// A bogus `pid` must fail `up` before any resource is created: no
/// project network, no container. The field-shaped error still names
/// the service and the field so the operator sees what to fix
/// (#1867, #1357).
#[tokio::test]
async fn up_rejects_invalid_pid_before_any_resource_is_created() {
	let fake = recording_fake();
	let e = engine_with(fake.client(), "probe");
	let file = parse_with_default(
		"services:\n  web:\n    image: alpine:latest\n    pid: \"evil\"\n    command: [\"sleep\", \"infinity\"]\n",
	);

	let err = e
		.up_with_options(&file, false, &[], &[], false, false, false, false)
		.await
		.expect_err("an invalid pid must fail up");
	let (service, field, value, message) =
		field_error(&err).expect("a field-shaped error must surface");
	assert_eq!(service, "web", "the error must name the service");
	assert_eq!(field, "pid", "the error must name the field");
	assert_eq!(value, "evil", "the error must name the offending value");
	assert!(
		message.contains("namespace mode"),
		"the error must explain why pid was rejected: {message}"
	);

	let creates = requests_creating(&fake);
	assert!(
		creates.is_empty(),
		"no resource may be created when pid is invalid, got: {creates:?}"
	);
}

/// The same shape, with `ipc` instead of `pid`. Both fields go through
/// the same `pre_validate_spec` namespace validator; pinning both
/// keeps a future narrowing of the validator from dropping one of
/// them silently (#1867).
#[tokio::test]
async fn up_rejects_invalid_ipc_before_any_resource_is_created() {
	let fake = recording_fake();
	let e = engine_with(fake.client(), "probe");
	let file = parse_with_default(
		"services:\n  web:\n    image: alpine:latest\n    ipc: \"bogus\"\n    command: [\"sleep\", \"infinity\"]\n",
	);

	let err = e
		.up_with_options(&file, false, &[], &[], false, false, false, false)
		.await
		.expect_err("an invalid ipc must fail up");
	let (service, field, value, _) = field_error(&err).expect("a field-shaped error must surface");
	assert_eq!(service, "web");
	assert_eq!(field, "ipc");
	assert_eq!(value, "bogus");

	let creates = requests_creating(&fake);
	assert!(
		creates.is_empty(),
		"no resource may be created when ipc is invalid, got: {creates:?}"
	);
}

/// Two services where the first is clean and the second is invalid.
/// A per-service check interleaved with container creation would
/// leave the first service's network behind (and possibly a
/// container); the up-front check fails the whole `up` with nothing
/// created. This is the case the per-service form gets wrong
/// (#1867).
#[tokio::test]
async fn up_with_two_services_rejects_the_second_without_creating_the_first() {
	let fake = recording_fake();
	let e = engine_with(fake.client(), "probe");
	let file = parse_with_default(
		"services:\n  clean:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n  bad:\n    image: alpine:latest\n    pid: \"evil\"\n    command: [\"sleep\", \"infinity\"]\n",
	);

	let err = e
		.up_with_options(&file, false, &[], &[], false, false, false, false)
		.await
		.expect_err("an invalid service must fail up");
	let (service, field, value, _) = field_error(&err).expect("a field-shaped error must surface");
	assert_eq!(service, "bad", "the error must name the offending service");
	assert_eq!(field, "pid");
	assert_eq!(value, "evil");

	let creates = requests_creating(&fake);
	assert!(
		creates.is_empty(),
		"neither service's resources may be created when one is invalid, got: {creates:?}"
	);
}
