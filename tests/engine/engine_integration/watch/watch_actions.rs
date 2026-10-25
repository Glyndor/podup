//! Watch integration tests for the actions a change triggers, driven through the
//! engine's test seams rather than the loop (require the test-helpers feature).
use super::*;
#[tokio::test]
async fn watch_no_develop_rules_errors() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("wno");
	let engine = Engine::new(client, proj.clone());
	// No develop.watch section → watch() errors, matching docker compose, which
	// reports "none of the selected services is configured for watch" rather than
	// silently exiting 0 (an invocation with nothing to watch is almost always a
	// misconfiguration the user wants flagged).
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	let err = engine
		.watch(&file)
		.await
		.expect_err("watch with no rules must error");
	assert!(
		matches!(err, podup::ComposeError::Watch(_)),
		"expected a Watch error, got {err:?}"
	);
}

#[tokio::test]
async fn watch_restart_container() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("wrs");
	let engine = Engine::new(client, proj.clone());
	// One line appended per start, on the container's own filesystem, which
	// survives a restart. That makes the line count a container-scoped record of
	// how many times the process started. See the note in
	// watch_sync_and_restart_does_both for why /proc/uptime is not an option.
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"echo start >> /starts; sleep infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	let cname = format!("{proj}-web-1");
	// Pin the fixture before acting. A counter that had not written its first
	// line yet, or that wrote more than one, would make the check below fail for
	// a reason that has nothing to do with restart.
	let started = poll_synced(&engine, &cname, "/starts", "start", 30).await;
	let before = engine
		.test_exec_capture(&cname, vec!["cat".into(), "/starts".into()])
		.await
		.unwrap_or_default();

	engine.test_watch_restart(&cname).await.unwrap();
	let restarted = poll_synced(&engine, &cname, "/starts", "start\nstart", 60).await;

	engine.down(&file).await.unwrap();
	assert!(started, "the container never wrote its first start line");
	assert_eq!(
		before.trim(),
		"start",
		"the fixture wrote more than one start line before the restart"
	);
	assert!(
		restarted,
		"restart returned success but the container process never started again"
	);
}

#[tokio::test]
async fn watch_exec_in_container() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("wex");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	let cname = format!("{proj}-web-1");
	// Write a marker to the container's filesystem instead of echoing to a stream
	// nobody reads: the action drains the exec's output and discards it, so an
	// echo leaves nothing an assertion can reach.
	engine
		.test_watch_exec(
			&cname,
			vec![
				"sh".to_string(),
				"-c".to_string(),
				"echo from-watch-exec >> /exec-ran".to_string(),
			],
		)
		.await
		.unwrap();
	let out = engine
		.test_exec_capture(&cname, vec!["cat".into(), "/exec-ran".into()])
		.await
		.unwrap_or_default();

	engine.down(&file).await.unwrap();
	// Exactly one line, so the action ran the command once and ran it in the
	// container rather than on the host.
	assert_eq!(
		out.trim(),
		"from-watch-exec",
		"the watch exec action did not run the command inside the container"
	);
}

#[tokio::test]
async fn watch_exec_surfaces_non_zero_exit() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("wee");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	let cname = format!("{proj}-web-1");
	let err = engine
		.test_watch_exec(&cname, vec!["sh".into(), "-c".into(), "exit 3".into()])
		.await
		.expect_err("a non-zero exit must surface as Err so the watch loop can log it");
	engine.down(&file).await.unwrap();
	assert!(
		err.to_string().contains("exited with status 3"),
		"the error must quote the captured exit code; got: {err}"
	);
}
