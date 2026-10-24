//! Resolved values and overrides must fail before later lifecycle work.

use crate::engine::{Engine, RunOptions};
use crate::libpod::Client;

fn overridden() -> Engine {
	Engine::new(Client::new("/nonexistent.sock"), "p".into()).with_up_overrides(
		Some("every_4294967296s".into()),
		false,
		false,
	)
}

fn file() -> crate::compose::types::ComposeFile {
	crate::parse_str("x-podman-pod: true\nservices:\n  web: {image: shared, pull_policy: daily, container_name: 'bad/name'}\n").unwrap()
}

fn override_error(result: crate::Result<()>) {
	let error = result.unwrap_err().to_string();
	assert!(
		error.contains("pull_policy") && error.contains("every_4294967296s"),
		"{error}"
	);
}

#[tokio::test]
async fn up_rejects_effective_override_before_pod_reconciliation() {
	override_error(overridden().up(&file()).await);
}

#[tokio::test]
async fn create_rejects_effective_override_before_pod_reconciliation() {
	override_error(
		overridden()
			.create_with_options(&file(), &[], &[], false, false, false)
			.await,
	);
}

#[tokio::test]
async fn run_rejects_effective_override_before_dependencies() {
	override_error(
		overridden()
			.run(&file(), "absent", RunOptions::default())
			.await,
	);
}

fn unresolved(field: &str, value: &str) {
	let file = crate::parse_str(&format!(
		"services:\n  web: {{image: shared, {field}: '{value}'}}\n"
	))
	.unwrap();
	let error = crate::validate_config(&file).unwrap_err().to_string();
	assert!(
		error.contains("web")
			&& error.contains(field)
			&& error.contains("variable did not resolve"),
		"{error}"
	);
}

#[test]
fn config_rejects_escaped_policy_variable() {
	unresolved("pull_policy", "every_$${UNRESOLVED}");
}

#[test]
fn config_rejects_escaped_refresh_variable() {
	unresolved("pull_refresh_after", "$${UNRESOLVED}");
}

#[test]
fn config_rejects_unresolved_tagged_values() {
	for field in ["pull_policy", "pull_refresh_after"] {
		let file = crate::parse_str(&format!(
			"services:\n  web:\n    image: shared\n    {field}: !override '$${{UNRESOLVED:-1h}}'\n"
		))
		.unwrap();
		let error = crate::validate_config(&file).unwrap_err().to_string();
		assert!(
			error.contains(field) && error.contains("variable did not resolve"),
			"{error}"
		);
	}
}
