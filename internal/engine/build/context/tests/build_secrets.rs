use super::*;

#[test]
fn extra_files_are_added_to_context_tar() {
	use flate2::read::GzDecoder;
	use std::io::Read;

	let dir = tempdir().unwrap();
	fs::write(dir.path().join("Dockerfile"), b"FROM alpine\n").unwrap();
	let extra = vec![(".podup-build-secret-tok".to_string(), b"hunter2".to_vec())];
	let bytes = build_context_tar(dir.path(), "Dockerfile", &extra).unwrap();

	let mut raw = Vec::new();
	GzDecoder::new(bytes.as_slice())
		.read_to_end(&mut raw)
		.unwrap();
	let mut archive = tar::Archive::new(raw.as_slice());
	let names: Vec<String> = archive
		.entries()
		.unwrap()
		.filter_map(|e| e.ok())
		.filter_map(|e| e.path().ok().map(|p| p.to_string_lossy().into_owned()))
		.collect();
	assert!(
		names.iter().any(|n| n.contains(".podup-build-secret-tok")),
		"secret entry must be present: {names:?}"
	);
}

#[test]
fn build_secret_entries_excluded_from_copy_via_dockerignore() {
	use flate2::read::GzDecoder;
	use std::io::Read;

	let dir = tempdir().unwrap();
	fs::write(dir.path().join("Dockerfile"), b"FROM alpine\nCOPY . /app\n").unwrap();
	fs::write(dir.path().join(".dockerignore"), b"*.log\n").unwrap();
	let extra = vec![(".podup-build-secret-tok".to_string(), b"hunter2".to_vec())];
	let bytes = build_context_tar(dir.path(), "Dockerfile", &extra).unwrap();

	let mut raw = Vec::new();
	GzDecoder::new(bytes.as_slice())
		.read_to_end(&mut raw)
		.unwrap();
	let mut archive = tar::Archive::new(raw.as_slice());
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
		di.lines().any(|l| l == ".podup-build-secret-tok"),
		"secret entry must be COPY-excluded via .dockerignore: {di:?}"
	);
}

#[test]
fn inline_tar_excludes_build_secrets_via_dockerignore() {
	use flate2::read::GzDecoder;
	use std::io::Read;

	let dir = tempdir().unwrap();
	fs::write(dir.path().join("app.txt"), b"x").unwrap();
	let extra = vec![(".podup-build-secret-db".to_string(), b"s3cret".to_vec())];
	let (bytes, _) = build_context_tar_with_inline(dir.path(), "FROM alpine\n", &extra).unwrap();

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
		"inline exclusion kept: {di:?}"
	);
	assert!(
		di.lines().any(|l| l == ".podup-build-secret-db"),
		"secret entry must be COPY-excluded via .dockerignore: {di:?}"
	);
}
