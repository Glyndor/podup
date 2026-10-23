//! Invalid pull intent must win over later errors, without opening a socket.

use crate::compose::types::ComposeFile;
use crate::engine::{BuildOptions, Engine, RunOptions};
use crate::error::ComposeError;
use crate::libpod::{Client, PodmanError};

fn engine() -> Engine {
	Engine::new(Client::new("/nonexistent.sock"), "p".into())
}

fn invalid() -> ComposeFile {
	crate::parse_str(
		"services:\n  web: {image: shared, pull_policy: every_5q, container_name: 'bad/name'}\n",
	)
	.unwrap()
}

fn policy_error(result: crate::error::Result<()>) {
	assert!(
		matches!(result, Err(ComposeError::Podman(PodmanError::Field { service, field, value, .. }))
		if service == "web" && field == "pull_policy" && value == "every_5q")
	);
}

#[tokio::test]
async fn up_rejects_pull_intent_before_resources() {
	policy_error(engine().up(&invalid()).await);
}

#[tokio::test]
async fn create_rejects_pull_intent_before_resources() {
	policy_error(
		engine()
			.create_with_options(&invalid(), &[], &[], false, false, false)
			.await,
	);
}

#[tokio::test]
async fn run_rejects_pull_intent_before_dependencies() {
	policy_error(
		engine()
			.run(&invalid(), "absent", RunOptions::default())
			.await,
	);
}

#[tokio::test]
async fn pull_rejects_pull_intent_before_dispatch() {
	policy_error(engine().pull_services(&invalid(), &["absent".into()]).await);
}

#[tokio::test]
async fn build_rejects_pull_intent_before_build_planning() {
	policy_error(
		engine()
			.build_all_with_options(&invalid(), &["absent".into()], &BuildOptions::default())
			.await,
	);
}

#[tokio::test]
async fn pod_recreation_rejects_pull_intent_first() {
	let mut file = invalid();
	file.extensions
		.insert("x-podman-pod".into(), serde_yaml::Value::Bool(true));
	policy_error(engine().up(&file).await);
}

#[test]
fn config_rejects_pull_intent() {
	policy_error(crate::validate_config(&invalid()));
}

#[test]
fn quadlet_rejects_pull_intent() {
	policy_error(crate::quadlet::validate_for_quadlet(&invalid()));
}

#[test]
fn quadlet_rejects_unused_refresh_interval() {
	let file = crate::parse_str(
		"services:\n  web: {image: shared, pull_policy: missing, pull_refresh_after: nonsense}\n",
	)
	.unwrap();
	let error = crate::quadlet::validate_for_quadlet(&file)
		.unwrap_err()
		.to_string();
	assert!(
		error.contains("web") && error.contains("pull_refresh_after") && error.contains("nonsense")
	);
}

#[test]
fn config_rejects_literal_pull_placeholders() {
	let file = crate::parse_str_raw("services:\n  web: {image: shared, pull_policy: '${POLICY}', pull_refresh_after: '${INTERVAL}'}\n").unwrap();
	let error = crate::validate_config(&file).unwrap_err().to_string();
	assert!(error.contains("variable did not resolve"));
	let json = serde_json::to_string(&file.services["web"]).unwrap();
	assert!(json.contains("${POLICY}") && json.contains("${INTERVAL}"));
}

#[tokio::test]
async fn run_acquires_the_image_with_the_effective_policy() {
	let mut e = engine();
	e.pull_policy_override = Some("invalid_override".into());
	let file = crate::parse_str("services:\n  web: {pull_policy: daily}\n").unwrap();
	let error = e
		.run(&file, "web", RunOptions::default())
		.await
		.unwrap_err();
	assert!(
		matches!(error, ComposeError::Podman(PodmanError::Field { field, value, .. })
		if field == "pull_policy" && value == "invalid_override")
	);
}
