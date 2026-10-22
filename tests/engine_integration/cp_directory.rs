//! `cp` integration tests for a directory payload, with and without the trailing
//! `/.` that changes where its contents land.
use std::fs;

use super::*;
/// I added this test because before #1777 was fixed a directory copy against
/// Podman 6 was reported as failed even when it landed on disk, which is why
/// the neighbour below could not assert on `cp`'s return value. I hold `cp`
/// to both halves here: the bytes arrive, and `cp` says so, on Podman 6 as on
/// Podman 5.
///
/// The payload holds a relative symlink (`link -> a.txt`) and a dangling one
/// (`dangling -> nowhere`), so the live proof that the link confirmation works
/// on the Podman 6 path is the `readlink` line below. A confirmation that
/// dropped the link would either fail outright on a Podman 6 that cannot
/// stat it or land but report `Err`, and `readlink` is the observable that
/// turns the difference into a measurable byte.
#[cfg(all(unix, feature = "test-helpers"))]
#[tokio::test]
async fn engine_cp_reports_a_directory_copy_as_landed() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	fs::create_dir(&payload).unwrap();
	fs::write(payload.join("a.txt"), b"first").unwrap();
	fs::write(payload.join("empty"), b"").unwrap();
	fs::create_dir(payload.join("nested")).unwrap();
	fs::write(payload.join("nested").join("b.txt"), b"second").unwrap();
	std::os::unix::fs::symlink("a.txt", payload.join("link")).unwrap();
	std::os::unix::fs::symlink("nowhere", payload.join("dangling")).unwrap();

	let proj = proj("cpdirland");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	engine.up(&file).await.unwrap();

	let result = engine
		.cp(&file, payload.to_str().unwrap(), "web:/tmp")
		.await;
	let out = engine
		.test_exec_capture(
			&format!("{proj}-web-1"),
			vec![
				"sh".into(),
				"-c".into(),
				"cat /tmp/payload/a.txt /tmp/payload/nested/b.txt && stat -c %s /tmp/payload/empty"
					.into(),
			],
		)
		.await;
	let readlink_out = engine
		.test_exec_capture(
			&format!("{proj}-web-1"),
			vec![
				"sh".into(),
				"-c".into(),
				"readlink /tmp/payload/link && readlink /tmp/payload/dangling".into(),
			],
		)
		.await;
	engine.down(&file).await.unwrap();

	let out = out.unwrap_or_default();
	assert!(
		out.contains("first"),
		"a.txt must arrive at /tmp/payload/a.txt, got {out:?}; cp returned {result:?}"
	);
	assert!(
		out.contains("second"),
		"nested/b.txt must arrive at /tmp/payload/nested/b.txt, got {out:?}; \
		 cp returned {result:?}"
	);
	assert!(
		out.contains("0\n"),
		"empty file must arrive at /tmp/payload/empty with size 0, got {out:?}; \
		 cp returned {result:?}"
	);
	let readlink_out = readlink_out.unwrap_or_default();
	assert!(
		readlink_out.contains("a.txt"),
		"relative symlink must arrive at /tmp/payload/link pointing at a.txt, got {readlink_out:?}; \
		 cp returned {result:?}"
	);
	assert!(
		readlink_out.contains("nowhere"),
		"dangling symlink must arrive at /tmp/payload/dangling pointing at nowhere, got {readlink_out:?}; \
		 cp returned {result:?}"
	);
	assert!(
		result.is_ok(),
		"a directory copy that landed (the bytes above prove it) must report \
		 Ok on Podman 6 as on Podman 5; got {result:?}"
	);
}

/// `cp host/. svc:/path` copies the directory's *contents* into the
/// container destination (matching `docker cp` / `podman cp`), so the file
/// ends up at `/tmp/a.txt` and not under `/tmp/payload/a.txt`. The trailing
/// `/.` is the cue, and `Path::new(src)` drops it: the engine has to detect
/// the cue on the original `src` string and thread it down to the packer.
#[cfg(all(unix, feature = "test-helpers"))]
#[tokio::test]
async fn engine_cp_with_dot_contents_copies_contents_into_destination() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	fs::create_dir(&payload).unwrap();
	fs::write(payload.join("a.txt"), b"contents-arrived").unwrap();

	let proj = proj("cpdot");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	engine.up(&file).await.unwrap();

	// `/.` is the cue; the rest of the path is unchanged from a plain
	// `payload` source.
	let result = engine
		.cp(
			&file,
			&format!("{}/.", payload.to_str().unwrap()),
			"web:/tmp",
		)
		.await;
	// The file must land directly in /tmp, not under /tmp/payload/. Each
	// probe prints a tag so a missing entry is observable on stdout, not
	// only on the shell exit code (which `test_exec_capture` does not
	// surface).
	let out = engine
		.test_exec_capture(
			&format!("{proj}-web-1"),
			vec![
				"sh".into(),
				"-c".into(),
				"echo TAG_A:$(cat /tmp/a.txt 2>/dev/null || echo MISSING); echo TAG_B:$(test -e /tmp/payload && echo PRESENT || echo ABSENT)".into(),
			],
		)
		.await;
	engine.down(&file).await.unwrap();

	assert!(
		result.is_ok(),
		"`cp payload/. svc:/tmp` must report Ok on Podman 6 as on Podman 5; got {result:?}"
	);
	let out = out.unwrap_or_default();
	assert!(
		out.contains("TAG_A:contents-arrived"),
		"contents must arrive at /tmp/a.txt, got {out:?}"
	);
	assert!(
		out.contains("TAG_B:ABSENT"),
		"there must be no /tmp/payload wrapper directory, got {out:?}"
	);
}

/// Today's behaviour, kept under test so a future change cannot quietly
/// regress it: `cp payload svc:/tmp` (without the `/.`) wraps the directory
/// under its own name, so the file lands at `/tmp/payload/a.txt` and not at
/// `/tmp/a.txt`. Held next to the `/.` case so the difference between the
/// two surfaces in the same test file.
#[cfg(all(unix, feature = "test-helpers"))]
#[tokio::test]
async fn engine_cp_without_dot_contents_wraps_under_source_basename() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	fs::create_dir(&payload).unwrap();
	fs::write(payload.join("a.txt"), b"under-basename").unwrap();

	let proj = proj("cpwrap");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	engine.up(&file).await.unwrap();

	let result = engine
		.cp(&file, payload.to_str().unwrap(), "web:/tmp")
		.await;
	// The file lands under the source's basename, not directly in /tmp.
	let out = engine
		.test_exec_capture(
			&format!("{proj}-web-1"),
			vec![
				"sh".into(),
				"-c".into(),
				"echo TAG_A:$(cat /tmp/payload/a.txt 2>/dev/null || echo MISSING); echo TAG_B:$(test -e /tmp/a.txt && echo PRESENT || echo ABSENT)".into(),
			],
		)
		.await;
	engine.down(&file).await.unwrap();

	assert!(
		result.is_ok(),
		"`cp payload svc:/tmp` must report Ok on Podman 6 as on Podman 5; got {result:?}"
	);
	let out = out.unwrap_or_default();
	assert!(
		out.contains("TAG_A:under-basename"),
		"contents must arrive at /tmp/payload/a.txt, got {out:?}"
	);
	assert!(
		out.contains("TAG_B:ABSENT"),
		"there must be no /tmp/a.txt (no /.), got {out:?}"
	);
}
