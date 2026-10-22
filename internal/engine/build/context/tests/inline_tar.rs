use super::*;

// build_context_tar_with_inline ----------------------------------------

#[test]
fn inline_tar_produces_valid_gzip() {
	let dir = tempdir().unwrap();
	fs::write(dir.path().join("app.txt"), b"content").unwrap();
	let inline = "FROM alpine\nRUN echo hello\n";
	let (bytes, df_name) = build_context_tar_with_inline(dir.path(), inline, &[]).unwrap();
	assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
	assert!(!df_name.is_empty());
}

#[test]
fn inline_dockerfile_excluded_via_dockerignore() {
	use flate2::read::GzDecoder;
	use std::io::Read;

	let dir = tempdir().unwrap();
	fs::write(dir.path().join("app.txt"), b"x").unwrap();
	let (bytes, _) = build_context_tar_with_inline(dir.path(), "FROM alpine\n", &[]).unwrap();

	let mut raw = Vec::new();
	GzDecoder::new(bytes.as_slice())
		.read_to_end(&mut raw)
		.unwrap();
	let mut archive = tar::Archive::new(raw.as_slice());
	let mut di = String::new();
	for entry in archive.entries().unwrap() {
		let mut entry = entry.unwrap();
		// No user ignore file in this context, so the synthesized one is
		// written as `.containerignore`, the name podman prefers, so a
		// stray `.dockerignore` cannot shadow our exclusions.
		if entry.path().unwrap().to_string_lossy() == ".containerignore" {
			entry.read_to_string(&mut di).unwrap();
		}
	}
	assert!(
		di.lines().any(|l| l == ".dockerfile-inline"),
		"inline Dockerfile must be excluded from COPY via .dockerignore: {di:?}"
	);
}

#[test]
fn inline_dockerignore_preserves_user_rules() {
	use flate2::read::GzDecoder;
	use std::io::Read;

	let dir = tempdir().unwrap();
	fs::write(dir.path().join("app.txt"), b"x").unwrap();
	fs::write(dir.path().join(".dockerignore"), b"*.log\n").unwrap();
	let (bytes, _) = build_context_tar_with_inline(dir.path(), "FROM alpine\n", &[]).unwrap();

	let mut raw = Vec::new();
	GzDecoder::new(bytes.as_slice())
		.read_to_end(&mut raw)
		.unwrap();
	let mut archive = tar::Archive::new(raw.as_slice());
	// The user's `.dockerignore` must appear exactly once, merged with the
	// synthesized inline-Dockerfile exclusion.
	let mut di_entries = 0;
	let mut di = String::new();
	for entry in archive.entries().unwrap() {
		let mut entry = entry.unwrap();
		if entry.path().unwrap().to_string_lossy() == ".dockerignore" {
			di_entries += 1;
			entry.read_to_string(&mut di).unwrap();
		}
	}
	assert_eq!(di_entries, 1, "exactly one .dockerignore entry");
	assert!(di.lines().any(|l| l == "*.log"), "user rule kept: {di:?}");
	assert!(
		di.lines().any(|l| l == ".dockerfile-inline"),
		"inline exclusion added: {di:?}"
	);
}
