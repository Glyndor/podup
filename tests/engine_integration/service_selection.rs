//! Engine integration tests for which services `up` starts: the ones a profile
//! filter leaves out, and the replicas a deploy count asks for.
use super::*;
// ---------------------------------------------------------------------------
// Profiles
// ---------------------------------------------------------------------------

#[tokio::test]
async fn profile_filtered_service_skipped() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("prf");
	let engine = Engine::new(client, proj.clone());
	// "debug" service has profile "debug", not in active profiles → skipped
	// "web" has no profiles → always runs
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n  debug:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    profiles: [\"debug\"]\n",
	)
	.unwrap();

	engine
		.up_with_options(&file, false, &[], &[], false, false, false, false)
		.await
		.unwrap();
	let mut names = engine
		.test_project_container_names()
		.await
		.unwrap_or_default();
	names.sort();
	engine.down(&file).await.unwrap();

	assert_eq!(
		names,
		vec![format!("{proj}-web-1")],
		"the profile-gated service was started even though its profile is not active"
	);
}

// ---------------------------------------------------------------------------
// Replicas
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scale_creates_multiple_replicas() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("rep");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  worker:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    deploy:\n      replicas: 2\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	let mut names = engine
		.test_project_container_names()
		.await
		.unwrap_or_default();
	names.sort();
	engine.down(&file).await.unwrap();

	// Both replicas, and the `-1`/`-2` suffixes rather than a bare base name: a
	// scaled service has no container under the base name, which is the shape
	// that made the readiness wait 404 in depends_on_scaled_service_completed.
	assert_eq!(
		names,
		vec![format!("{proj}-worker-1"), format!("{proj}-worker-2")],
		"replicas: 2 did not produce two suffixed containers"
	);
}
