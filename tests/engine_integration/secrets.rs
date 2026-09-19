//! Reproduction of the file-edit / recreated-container contract for `file:`
//! secrets. The unit tests pin the hash; this pins the end-to-end behaviour
//! on a real Podman: a service started against `s.txt = v1` must see `v2`
//! after the file is rewritten and `up -d` runs again. Before the fix in
//! `internal/engine/container/resolve.rs` the second `up` left the original
//! container running and the new bytes invisible to the container. The
//! fix folds the file content into the config hash so the second `up`
//! recreates the container.

use super::*;

/// Run the steps the issue measured. The marker technique separates the two
/// `up` paths the same way `up_no_recreate_skips_running` does, but here
/// the marker is the secret itself: the secret is read-only by the container,
/// so its bytes only change when the container is recreated and the new
/// Podman-native secret replaces the old one. Skipping the recreate keeps
/// the old bytes; recreating drops the old mount and remounts the new one.
///
/// `v1_first` and `v2_after` are both strict: a missing read is a recreate
/// failure, not a silent skip. The assertion after `engine.down` compares
/// the trimmed bytes against the literal we wrote.
#[tokio::test]
async fn changing_a_file_secret_recreates_with_the_new_bytes() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let secret_path = dir.path().join("s.txt");
	std::fs::write(&secret_path, b"v1").unwrap();

	let proj = proj("fsec-rot");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let yaml = format!(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    secrets:\n      - s\nsecrets:\n  s:\n    file: {}\n",
		secret_path.display()
	);
	let file = parse_str(&yaml).unwrap();

	engine.up(&file).await.unwrap();
	let cname = format!("{proj}-web-1");
	let v1_first = engine
		.test_exec_capture(&cname, vec!["cat".into(), "/run/secrets/s".into()])
		.await
		.unwrap_or_default();

	std::fs::write(&secret_path, b"v2").unwrap();
	engine.up(&file).await.unwrap();
	let v2_after = engine
		.test_exec_capture(&cname, vec!["cat".into(), "/run/secrets/s".into()])
		.await
		.unwrap_or_default();
	engine.down(&file).await.unwrap();

	assert_eq!(
		v1_first.trim(),
		"v1",
		"the first up did not carry v1 to the container, so the test cannot prove the second up did anything"
	);
	assert_eq!(
		v2_after.trim(),
		"v2",
		"up -d after the file was rewritten kept the old container instead of recreating with the new bytes"
	);
}
