//! Pod recreation cannot precede mask validation of its members.

#[tokio::test]
async fn umask_pod_recreation_rejects_before_transport() {
	let mut file =
		crate::parse_str("services:\n  worker:\n    image: alpine\nx-podman-pod: true\n").unwrap();
	file.services["worker"].umask = Some("0022 --privileged".into());
	let engine = crate::engine::Engine::with_base_dir(
		crate::libpod::Client::closed_for_preflight(),
		"probe".into(),
		std::env::temp_dir(),
	);
	let error = engine.ensure_pod(&file, &[]).await.unwrap_err().to_string();
	assert!(
		error.contains("worker") && error.contains("umask"),
		"{error}"
	);
}
