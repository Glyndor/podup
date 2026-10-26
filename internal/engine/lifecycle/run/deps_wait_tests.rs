//! `run` waits on the conditions its target states about its dependencies,
//! for a service and for a job alike.

use crate::engine::fake_podman;
use crate::engine::lifecycle::RunOverrides;
use crate::engine::Engine;
use crate::error::ComposeError;

fn file(condition: &str) -> String {
	format!(
		"services:\n  prepare:\n    image: prepare-image\n  publish:\n    image: publish-image\n    depends_on:\n      prepare:\n        condition: {condition}\njobs:\n  release:\n    image: release-image\n    depends_on:\n      prepare:\n        condition: {condition}\n    triggers: {{manual: true}}\n"
	)
}

fn options() -> crate::engine::RunOptions {
	crate::engine::RunOptions {
		cmd: vec![],
		rm: false,
		detach: true,
		env_overrides: vec![],
		name_override: None,
		service_ports: false,
	}
}

/// A fake in which waiting for the container `prepare` ends with `exit_code`.
fn engine(exit_code: i64) -> (fake_podman::FakePodman, Engine) {
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/images/") {
			(200, "{}".to_string())
		} else if method == "POST" && target.contains("/wait") {
			(200, exit_code.to_string())
		} else if method == "POST" {
			(200, r#"{"Id":"cafe"}"#.to_string())
		} else if method == "GET" && target.contains("/containers/json") {
			(200, "[]".to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	(fake, engine)
}

fn created(fake: &fake_podman::FakePodman, image: &str) -> bool {
	fake.bodies
		.lock()
		.unwrap()
		.iter()
		.any(|body| String::from_utf8_lossy(body).contains(image))
}

fn waited(fake: &fake_podman::FakePodman) -> bool {
	fake.requests
		.lock()
		.unwrap()
		.iter()
		.any(|request| request.contains("/proj-prepare-1/wait"))
}

#[tokio::test]
async fn a_service_is_not_created_while_its_dependency_failed() {
	let (fake, engine) = engine(3);
	let file = crate::parse_str(&file("service_completed_successfully")).unwrap();

	let err = engine
		.run(&file, "publish", options())
		.await
		.expect_err("prepare exited with 3");
	assert!(
		matches!(err, ComposeError::HealthCheckTimeout(_)),
		"{err:?}"
	);
	assert!(
		!created(&fake, "publish-image"),
		"publish must not be created"
	);
}

#[tokio::test]
async fn a_job_is_not_created_while_its_dependency_failed() {
	let (fake, engine) = engine(3);
	let file = crate::parse_str(&file("service_completed_successfully")).unwrap();

	let err = engine
		.run(&file, "release", options())
		.await
		.expect_err("prepare exited with 3");
	assert!(
		matches!(err, ComposeError::HealthCheckTimeout(_)),
		"{err:?}"
	);
	assert!(!created(&fake, "release-image"));
}

#[tokio::test]
async fn the_target_is_created_once_the_dependency_completed() {
	let (fake, engine) = engine(0);
	let file = crate::parse_str(&file("service_completed_successfully")).unwrap();

	engine.run(&file, "publish", options()).await.unwrap();
	assert!(waited(&fake), "the condition was waited on");
	assert!(created(&fake, "publish-image"));
}

#[tokio::test]
async fn a_started_condition_waits_for_nothing() {
	let (fake, engine) = engine(3);
	let file = crate::parse_str(&file("service_started")).unwrap();

	engine.run(&file, "publish", options()).await.unwrap();
	assert!(!waited(&fake));
	assert!(created(&fake, "publish-image"));
}

#[tokio::test]
async fn an_optional_dependency_that_failed_does_not_stop_the_run() {
	let (fake, engine) = engine(3);
	let yaml = file("service_completed_successfully").replace(
		"        condition: service_completed_successfully\njobs:",
		"        condition: service_completed_successfully\n        required: false\njobs:",
	);
	let file = crate::parse_str(&yaml).unwrap();

	engine.run(&file, "publish", options()).await.unwrap();
	assert!(waited(&fake));
	assert!(created(&fake, "publish-image"));
}

#[tokio::test]
async fn no_deps_starts_and_waits_for_nothing() {
	let (fake, engine) = engine(3);
	let engine = engine.with_run_overrides(RunOverrides {
		no_deps: true,
		..Default::default()
	});
	let file = crate::parse_str(&file("service_completed_successfully")).unwrap();

	engine.run(&file, "publish", options()).await.unwrap();
	assert!(!waited(&fake));
	assert!(
		!created(&fake, "prepare-image"),
		"the dependency was not started"
	);
	assert!(created(&fake, "publish-image"));
}

#[tokio::test]
async fn a_healthy_condition_on_a_disabled_healthcheck_waits_for_nothing() {
	let (fake, engine) = engine(3);
	let yaml = file("service_healthy").replace(
		"    image: prepare-image\n",
		"    image: prepare-image\n    healthcheck: {disable: true}\n",
	);
	let file = crate::parse_str(&yaml).unwrap();

	engine.run(&file, "publish", options()).await.unwrap();
	assert!(!waited(&fake));
	assert!(created(&fake, "publish-image"));
}
