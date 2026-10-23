//! Engine integration tests for when `up` recreates a container and when it leaves it
//! alone.
use super::*;
// ---------------------------------------------------------------------------

#[tokio::test]
async fn up_skips_recreate_when_config_unchanged() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("rch");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	let cname = format!("{proj}-web-1");
	// A recreate builds a fresh filesystem under the same container name, so a
	// marker written into the old one is what tells the three outcomes apart.
	// Every phase below returned success before this test asserted anything, so
	// nothing here distinguished "skipped" from "recreated".
	let write_marker = vec![
		"sh".to_string(),
		"-c".to_string(),
		"echo marked > /marker".to_string(),
	];
	let read_marker = vec!["cat".to_string(), "/marker".to_string()];
	engine
		.test_exec_capture(&cname, write_marker.clone())
		.await
		.unwrap();

	// Same config again -> config-hash matches -> skip recreate + ensure started.
	engine.up(&file).await.unwrap();
	let after_same = engine
		.test_exec_capture(&cname, read_marker.clone())
		.await
		.unwrap_or_default();

	// force_recreate -> recreate even though config is unchanged.
	engine
		.up_with_options(&file, false, &[], &[], false, true, false, false)
		.await
		.unwrap();
	let after_force = engine
		.test_exec_capture(&cname, read_marker.clone())
		.await
		.unwrap_or_default();

	// Changed config -> hash differs -> recreate.
	engine
		.test_exec_capture(&cname, write_marker)
		.await
		.unwrap();
	let file2 = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"120\"]\n",
	)
	.unwrap();
	engine.up(&file2).await.unwrap();
	let after_change = engine
		.test_exec_capture(&cname, read_marker)
		.await
		.unwrap_or_default();

	engine.down(&file2).await.unwrap();
	assert_eq!(
		after_same.trim(),
		"marked",
		"an unchanged config recreated the container instead of skipping it"
	);
	assert_ne!(
		after_force.trim(),
		"marked",
		"force_recreate skipped the container instead of recreating it"
	);
	assert_ne!(
		after_change.trim(),
		"marked",
		"a changed config skipped the container instead of recreating it"
	);
}
