//! A variable `environment:` names without a value, and nothing resolves, is
//! removed from the container, including the value the image gives it.

use super::*;

/// The image sets `VALKEY_VERSION`; it is small and already what other suites
/// pull. The service that names the variable bare must not see it, and the one
/// that does not name it must, which keeps the check from passing on an image
/// that never had the variable.
#[cfg(feature = "test-helpers")]
#[tokio::test]
async fn a_bare_unresolved_key_removes_the_image_value() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("unsetenv");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  removed:\n    image: docker.io/valkey/valkey:8-alpine\n    command: [\"sleep\", \"300\"]\n    environment:\n      - VALKEY_VERSION\n      - KEPT=1\n  control:\n    image: docker.io/valkey/valkey:8-alpine\n    command: [\"sleep\", \"300\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	let removed = engine
		.test_exec_capture(&format!("{proj}-removed-1"), vec!["env".into()])
		.await;
	let control = engine
		.test_exec_capture(&format!("{proj}-control-1"), vec!["env".into()])
		.await;
	engine.down(&file).await.unwrap();

	let removed = removed.expect("exec in removed failed");
	let control = control.expect("exec in control failed");
	assert!(
		control.contains("VALKEY_VERSION="),
		"the image does not set the variable, so the check proves nothing: {control}"
	);
	assert!(
		!removed.contains("VALKEY_VERSION="),
		"the image's value was kept: {removed}"
	);
	assert!(removed.contains("KEPT=1"), "{removed}");
}
