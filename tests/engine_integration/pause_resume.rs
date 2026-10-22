//! Engine integration tests for `pause` and `unpause`.
use super::*;
// ---------------------------------------------------------------------------
// Pause / unpause
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pause_and_unpause() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("pau");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	let cname = format!("{proj}-web-1");
	// A paused container refuses new exec sessions: Podman answers "can only
	// create exec sessions on running containers". That refusal is the observable
	// difference between a pause that happened and one that returned Ok having
	// done nothing, which is all this test used to check.
	let before = engine.test_exec_capture(&cname, vec!["true".into()]).await;
	engine.pause(&file, &[]).await.unwrap();
	let while_paused = engine.test_exec_capture(&cname, vec!["true".into()]).await;
	engine.unpause(&file, &[]).await.unwrap();
	let after = engine.test_exec_capture(&cname, vec!["true".into()]).await;
	engine.down(&file).await.unwrap();

	assert!(
		before.is_ok(),
		"the container did not accept an exec before being paused: {before:?}"
	);
	assert!(
		while_paused.is_err(),
		"pause returned success but the container still accepted an exec"
	);
	assert!(
		after.is_ok(),
		"unpause did not return the container to running: {after:?}"
	);
}
