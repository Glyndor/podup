//! Engine integration tests for `cp` in both directions, and for a host archive
//! large enough to need the whole of the buffer.
use std::fs;

use super::*;
// ---------------------------------------------------------------------------
// Cp
// ---------------------------------------------------------------------------

#[tokio::test]
async fn engine_cp_from_container_extracts_file() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let proj = proj("cpf");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();

	let dst = dir.path().to_str().unwrap().to_string();
	let src = "web:/etc/hostname".to_string();
	let result = engine.cp(&file, &src, &dst).await;
	engine.down(&file).await.unwrap();

	result.unwrap();
	assert!(dir.path().join("hostname").exists());
}

#[tokio::test]
async fn engine_cp_to_container_uploads_file() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let local_file = dir.path().join("testfile.txt");
	fs::write(&local_file, b"hello from host").unwrap();

	let proj = proj("cpt");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();

	let src = local_file.to_str().unwrap().to_string();
	let dst = "web:/tmp".to_string();
	let result = engine.cp(&file, &src, &dst).await;
	// `cp` reporting success is not the same as the file arriving: that exact
	// false success is what #1097 was on Podman 6, where libpod accepted the
	// archive, closed the connection without a response, and podup called it done.
	// Read the copy back out of the container before tearing anything down.
	let landed = engine
		.exec_with_options(
			&file,
			"web",
			vec![
				"sh".to_string(),
				"-c".to_string(),
				"test \"$(cat /tmp/testfile.txt)\" = 'hello from host'".to_string(),
			],
			podup::ExecOptions::default(),
		)
		.await;
	engine.down(&file).await.unwrap();

	result.unwrap();
	assert!(
		landed.is_ok(),
		"cp reported success but /tmp/testfile.txt is missing or has the wrong contents: {landed:?}"
	);
}

/// The container-to-host archive is buffered in memory under a cap of one
/// gibibyte. A file of two mebibytes is far under it and must arrive whole;
/// a cap that shrank by an arithmetic slip (a mutation sweep on 2026-09-02
/// turned the `*` in the constant into `+` and nothing noticed, because every
/// copy in the suite was a few bytes) refuses this one.
#[tokio::test]
async fn engine_cp_from_container_carries_a_file_of_two_mebibytes() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("cpbig");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	engine.up(&file).await.unwrap();
	engine
		.test_exec_capture(
			&format!("{proj}-web-1"),
			vec![
				"sh".into(),
				"-c".into(),
				"head -c 2097152 /dev/zero > /tmp/big.bin".into(),
			],
		)
		.await
		.unwrap();

	let dir = tempfile::tempdir().unwrap();
	let dst = dir.path().to_str().unwrap().to_string();
	let result = engine.cp(&file, "web:/tmp/big.bin", &dst).await;
	engine.down(&file).await.unwrap();
	result.unwrap();
	let size = fs::metadata(dir.path().join("big.bin")).unwrap().len();
	assert_eq!(size, 2 * 1024 * 1024, "the whole file must arrive");
}
