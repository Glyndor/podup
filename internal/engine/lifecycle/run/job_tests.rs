//! `run` with the name of a job: the job runs through the one-off machinery of
//! a service, and a job that cannot be launched by hand is refused before the
//! engine is asked for anything.

use crate::engine::fake_podman;
use crate::engine::Engine;
use crate::error::ComposeError;

const FILE: &str = "\
services:
  web:
    image: web-image
jobs:
  nightly:
    image: job-image
    command: [echo, hi]
    triggers:
      manual: true
  closed:
    image: closed-image
    triggers:
      manual: false
      schedule: ['0 3 * * *']
";

fn options() -> crate::engine::RunOptions {
	crate::engine::RunOptions {
		cmd: vec![],
		rm: false,
		// Detached, so the run returns as soon as the container is started.
		detach: true,
		env_overrides: vec![],
		name_override: None,
		service_ports: false,
	}
}

fn engine() -> (fake_podman::FakePodman, Engine) {
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/images/") {
			(200, "{}".to_string())
		} else if method == "POST" {
			(200, r#"{"Id":"cafe"}"#.to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	(fake, engine)
}

#[tokio::test]
async fn a_job_runs_as_a_one_off_container_of_its_own_image() {
	let (fake, engine) = engine();
	let file = crate::parse_str(FILE).unwrap();

	engine
		.run(&file, "nightly", options())
		.await
		.expect("a detached run of a manual job succeeds");

	let bodies = fake.bodies.lock().unwrap();
	let created = bodies
		.iter()
		.map(|b| String::from_utf8_lossy(b).into_owned())
		.find(|b| b.contains("job-image"))
		.expect("a container was created from the job's image");
	assert!(created.contains("nightly"), "named for the job: {created}");
	let seen = fake.requests.lock().unwrap();
	assert!(
		!seen.iter().any(|r| r.contains("web-image")),
		"the service next to the job is not touched: {seen:?}"
	);
}

#[tokio::test]
async fn a_job_with_manual_false_is_refused_before_the_engine_is_asked() {
	let (fake, engine) = engine();
	let file = crate::parse_str(FILE).unwrap();

	let err = engine
		.run(&file, "closed", options())
		.await
		.expect_err("manual: false forbids `run`");
	assert!(matches!(err, ComposeError::Job(_)), "{err:?}");
	assert!(
		fake.requests.lock().unwrap().is_empty(),
		"nothing may reach the engine: {:?}",
		fake.requests.lock().unwrap()
	);
}

#[tokio::test]
async fn a_name_that_is_neither_service_nor_job_is_not_found() {
	let (_fake, engine) = engine();
	let file = crate::parse_str(FILE).unwrap();

	let err = engine
		.run(&file, "ghost", options())
		.await
		.expect_err("no such service or job");
	assert!(matches!(err, ComposeError::ServiceNotFound(_)), "{err:?}");
}
