//! `cp` flag-parity integration tests (split for the source line limit).
use super::*;

#[tokio::test]
async fn engine_cp_index_out_of_range_errors() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let local = dir.path().join("f.txt");
	fs::write(&local, b"x").unwrap();

	let proj = proj("cpidx");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str("services:\n  web:\n    image: alpine:latest\n").unwrap();

	// No --index target replica 9 of a single-replica service: must error rather
	// than silently fall back to the first container.
	let result = engine
		.cp_with_options(
			&file,
			local.to_str().unwrap(),
			"web:/tmp",
			podup::CpOptions {
				index: Some(9),
				..Default::default()
			},
		)
		.await;
	assert!(
		matches!(result, Err(podup::ComposeError::ServiceNotFound(_))),
		"out-of-range --index must error, got {result:?}"
	);
}

#[cfg(feature = "test-helpers")]
#[tokio::test]
async fn engine_cp_follow_link_uploads_target_contents() {
	use std::os::unix::fs::symlink;
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("target.txt");
	fs::write(&target, b"linked-content").unwrap();
	let link = dir.path().join("link.txt");
	symlink(&target, &link).unwrap();

	let proj = proj("cplnk");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	engine.up(&file).await.unwrap();

	// -L follows the host symlink, so the container receives the target's bytes
	// (a regular file), not a dangling link.
	let result = engine
		.cp_with_options(
			&file,
			link.to_str().unwrap(),
			"web:/tmp",
			podup::CpOptions {
				follow_link: true,
				..Default::default()
			},
		)
		.await;
	let out = engine
		.test_exec_capture(
			&format!("{proj}-web"),
			vec!["cat".into(), "/tmp/link.txt".into()],
		)
		.await;
	engine.down(&file).await.unwrap();
	result.unwrap();
	assert!(
		out.unwrap_or_default().contains("linked-content"),
		"-L must upload the symlink target's contents"
	);
}
