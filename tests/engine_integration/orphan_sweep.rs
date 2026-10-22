//! Engine integration tests for what the orphan sweep must leave alone when there is
//! no orphan to remove.
use super::*;
#[tokio::test]
async fn remove_orphans_no_orphans() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("orp");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.remove_orphans(&file).await.unwrap();
	// The point of this test is what remove_orphans must NOT do. With no orphan
	// present, a sweep that removed the project's own container would still have
	// returned success.
	let survivors = engine
		.test_project_container_names()
		.await
		.unwrap_or_default();
	engine.down(&file).await.unwrap();

	assert_eq!(
		survivors,
		vec![format!("{proj}-web-1")],
		"remove_orphans removed a container that belongs to the project"
	);
}
