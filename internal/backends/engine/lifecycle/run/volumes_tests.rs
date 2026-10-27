//! The volumes a one-off `run` needs before it creates its container.
//!
//! `up` pre-creates every declared volume before it creates a container, so
//! the container mounts a volume that carries the project's label and the
//! declared driver options. `run` created the container without that step:
//! the runtime auto-created a bare volume instead, without the options or the
//! labels, and an `external: true` volume was never checked for existence —
//! the check `up` performs and the compose spec requires.

use crate::engine::fake_podman::{self, FakePodman};
use crate::engine::Engine;

/// The compose file under test: a service that mounts a named volume.
const NAMED_VOLUME: &str = "\
services:
  app:
    image: img
    volumes:
      - datos:/data
volumes:
  datos:
    driver_opts:
      type: tmpfs
      device: tmpfs
";

/// The same, with a volume the project does not own.
const EXTERNAL_VOLUME: &str = "\
services:
  app:
    image: img
    volumes:
      - shared:/data
volumes:
  shared:
    external: true
    name: shared_data
";

fn options() -> crate::engine::RunOptions {
	crate::engine::RunOptions {
		cmd: vec![],
		rm: false,
		// Detached, so the run returns as soon as the container is started and
		// the test never has to model a log stream.
		detach: true,
		env_overrides: vec![],
		name_override: None,
		service_ports: false,
	}
}

/// A fake that answers the image inspect, the volume inspect and every POST.
fn engine(volume_exists: bool) -> (FakePodman, Engine) {
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/images/") {
			(200, "{}".to_string())
		} else if method == "GET" && target.contains("/volumes/") {
			if volume_exists {
				(200, "{}".to_string())
			} else {
				(404, r#"{"message":"no such volume"}"#.to_string())
			}
		} else if method == "POST" {
			(200, r#"{"Id":"cafe"}"#.to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	(fake, engine)
}

/// The body of the `POST /volumes/create` the fake was asked to send, if any.
fn volume_create_body(fake: &FakePodman) -> Option<serde_json::Value> {
	let requests = fake.requests.lock().unwrap();
	let bodies = fake.bodies.lock().unwrap();
	let index = requests
		.iter()
		.position(|r| r.contains("/volumes/create"))?;
	Some(serde_json::from_slice(&bodies[index]).expect("the create payload is JSON"))
}

/// The position of the first request whose line contains `needle`.
fn first(fake: &FakePodman, needle: &str) -> Option<usize> {
	let requests = fake.requests.lock().unwrap();
	requests.iter().position(|r| r.contains(needle))
}

/// A named volume is created with the project's label and the declared driver
/// options, before the container that mounts it.
#[tokio::test]
async fn a_run_creates_the_named_volume_it_mounts() {
	let (fake, engine) = engine(true);
	let file = crate::parse_str(NAMED_VOLUME).expect("the fixture parses");

	engine
		.run(&file, "app", options())
		.await
		.expect("a detached run against the fake succeeds");

	let body = volume_create_body(&fake).expect("the run must create the volume");
	assert_eq!(body["name"], "proj_datos");
	assert_eq!(body["driver"], "local");
	assert_eq!(body["Options"]["type"], "tmpfs");
	assert_eq!(body["Options"]["device"], "tmpfs");
	assert_eq!(body["labels"]["podup.project"], "proj");

	let volume_at = first(&fake, "/volumes/create").expect("the volume create happened");
	let container_at = first(&fake, "/containers/create").expect("the container create happened");
	assert!(
		volume_at < container_at,
		"the volume must exist before the container that mounts it"
	);
}

/// An external volume is checked for existence, and a missing one stops the
/// run instead of leaving the container to fail later with an opaque mount
/// error.
#[tokio::test]
async fn a_run_refuses_a_missing_external_volume() {
	let (fake, engine) = engine(false);
	let file = crate::parse_str(EXTERNAL_VOLUME).expect("the fixture parses");

	let error = engine
		.run(&file, "app", options())
		.await
		.expect_err("a missing external volume must fail the run");

	assert!(
		matches!(&error, crate::error::ComposeError::ExternalNotFound(_)),
		"the missing external volume must surface as such, got {error:?}"
	);
	assert!(
		first(&fake, "/containers/create").is_none(),
		"no container is created when the volume it mounts does not exist"
	);
}
