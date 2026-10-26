//! Rebuild validation precedes build, secrets and container replacement.

#[tokio::test]
async fn umask_watch_rebuild_rejects_before_transport() {
	let mut file =
		crate::parse_str("services:\n  worker:\n    image: alpine\n    build: .\n").unwrap();
	file.services["worker"].umask = Some("${MASK}".into());
	let engine = crate::engine::Engine::with_base_dir(
		crate::libpod::Client::closed_for_preflight(),
		"probe".into(),
		std::env::temp_dir(),
	);
	let error = engine
		.watch_rebuild(&file, "worker")
		.await
		.unwrap_err()
		.to_string();
	assert!(
		error.contains("worker") && error.contains("umask"),
		"{error}"
	);
}
