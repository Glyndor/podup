//! Engine integration tests for `up` restricted to the services it was given.
use super::*;
// ---------------------------------------------------------------------------
// Lifecycle: target_set skips non-targeted service; dep profile skip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn target_services_skips_non_dep() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("tsk");
	let engine = Engine::new(client, proj.clone());
	// "extra" is not depended upon by web → skipped (lifecycle.rs L56 continue)
	let file = parse_str(
		"services:\n  extra:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine
		.up_with_options(
			&file,
			false,
			&[],
			&["web".to_string()],
			false,
			false,
			false,
			false,
		)
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
		"targeting web started extra as well, which nothing depends on"
	);
}
