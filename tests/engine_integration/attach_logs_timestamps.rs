//! Engine integration tests for `attach_logs --timestamps` returning once the
//! container it follows has exited.
use super::*;
#[tokio::test]
async fn engine_attach_logs_timestamps_returns_when_container_exits() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("ats");
	let engine = Engine::new(client, proj.clone());
	// A short-lived container: its follow log stream closes on exit, so
	// attach_logs returns instead of blocking.
	let file =
		parse_str("services:\n  job:\n    image: alpine:latest\n    command: [\"echo\", \"hi\"]\n")
			.unwrap();

	engine.up(&file).await.unwrap();
	let res = tokio::time::timeout(
		std::time::Duration::from_secs(20),
		engine.attach_logs_with_options(&file, true),
	)
	.await;
	engine.down(&file).await.unwrap();
	assert!(
		res.is_ok(),
		"attach_logs --timestamps did not return for an exited container"
	);
	res.unwrap().unwrap();
}
