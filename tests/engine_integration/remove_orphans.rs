//! Engine integration tests for the orphan sweep taking a container out of the
//! project.
use super::*;
// ---------------------------------------------------------------------------
// Orphan removal
// ---------------------------------------------------------------------------

#[tokio::test]
async fn remove_orphans_removes_container() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("orr");
	let engine = Engine::new(client, proj.clone());

	let file_svc1 = parse_str(
		"services:\n  svc1:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	engine.up(&file_svc1).await.unwrap();

	// file_svc2 only declares svc2, so svc1 becomes an orphan
	let file_svc2 = parse_str(
		"services:\n  svc2:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	engine.remove_orphans(&file_svc2).await.unwrap();
	let survivors = engine
		.test_project_container_names()
		.await
		.unwrap_or_default();

	// cleanup (svc1 already removed; down() on either file is a no-op for missing containers)
	let _ = engine.down(&file_svc1).await;

	// The sibling test `remove_orphans_no_orphans` pins that a sweep with no orphan
	// removes nothing. This one is the other direction, and until now neither
	// looked: a sweep that removed nothing at all satisfied both.
	assert!(
		!survivors.iter().any(|n| n.contains("-svc1-")),
		"the orphaned svc1 container survived the sweep: {survivors:?}"
	);
}
