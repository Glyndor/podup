//! Engine integration tests for the `post_start` and `pre_stop` hooks.
use std::fs;

use super::*;
// ---------------------------------------------------------------------------
// Lifecycle hooks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn post_start_and_pre_stop_hooks_run() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("hks");
	let dir = tempfile::tempdir().unwrap();
	let engine = Engine::new(client, proj.clone());
	// The hooks used to `echo` into a stream nothing reads, so a hook that never
	// ran looked the same as one that did. They append to a bind-mounted host
	// directory instead, which is also the only way to observe `pre_stop`: it
	// fires while the container is going away, so there is nothing left to exec
	// into afterwards. The `z` relabel is required on an SELinux-enforcing host
	// (the lane is Fedora); without it the container is denied the write.
	let yaml = format!(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    volumes:\n      - {out}:/out:z\n    post_start:\n      - command: [\"sh\", \"-c\", \"echo post-start >> /out/hooks\"]\n    pre_stop:\n      - command: [\"sh\", \"-c\", \"echo pre-stop >> /out/hooks\"]\n",
		out = dir.path().display()
	);
	let file = parse_str(&yaml).unwrap();

	engine.up(&file).await.unwrap();
	let after_up = fs::read_to_string(dir.path().join("hooks")).unwrap_or_default();
	engine.down(&file).await.unwrap();
	let after_down = fs::read_to_string(dir.path().join("hooks")).unwrap_or_default();

	assert_eq!(
		after_up.trim(),
		"post-start",
		"post_start did not run, or ran more than once"
	);
	assert_eq!(
		after_down.trim(),
		"post-start\npre-stop",
		"pre_stop did not run on down"
	);
}
