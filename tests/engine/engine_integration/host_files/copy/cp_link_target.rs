//! The stat of a symbolic link carries a `linkTarget` on Podman 6.2.0 for every
//! kind of link. The upload verification confirms a link by that target and
//! refuses one whose stat has none, so this is the premise it stands on, read
//! from the real runtime.

use super::*;

#[cfg(all(unix, feature = "test-helpers"))]
#[tokio::test]
async fn every_kind_of_symlink_stats_with_a_link_target() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("linktarget");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: busybox:latest\n    command: [\"sh\", \"-c\", \"mkdir -p /tmp/d && cd /tmp/d && ln -s ../etc/hosts rel && ln -s /etc/hosts abs && ln -s /nonexistent/x dangling && ln -s /etc todir && ln -s /tmp/d/abs chain && touch ready && sleep 300\"]\n",
	)
	.unwrap();
	let container = format!("{proj}-web-1");

	engine.up(&file).await.unwrap();
	let mut ready = false;
	for _ in 0..100 {
		if let Ok(out) = engine
			.test_exec_capture(&container, vec!["ls".into(), "/tmp/d/ready".into()])
			.await
		{
			if out.contains("/tmp/d/ready") {
				ready = true;
				break;
			}
		}
		tokio::time::sleep(std::time::Duration::from_millis(100)).await;
	}
	let mut stats = Vec::new();
	for name in ["rel", "abs", "dangling", "todir", "chain"] {
		stats.push((
			name,
			engine
				.test_link_target(&container, &format!("/tmp/d/{name}"))
				.await,
		));
	}
	engine.down(&file).await.unwrap();

	assert!(ready, "the links were never made");
	for (name, target) in stats {
		let target = target
			.unwrap_or_else(|e| panic!("{name}: the stat failed: {e}"))
			.unwrap_or_else(|| panic!("{name}: no stat in the response"));
		assert!(
			!target.is_empty(),
			"{name}: the runtime sent no linkTarget, which the upload verification refuses"
		);
	}
}
