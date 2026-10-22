use super::*;

#[test]
fn pack_path_single_file() {
	let dir = tempfile::tempdir().expect("tempdir");
	let file = dir.path().join("data.txt");
	std::fs::write(&file, b"hello").expect("write");
	let bytes = pack_to_vec(&file, false, None, false);
	assert!(!bytes.is_empty());
}

/// The local socket carries plain tar, not a gzip stream. Read the bytes
/// themselves rather than trusting the constructor: a gzip stream starts with
/// the magic header `0x1f 0x8b`, and the tar header is the entry's name as
/// ASCII.
#[test]
fn pack_path_archive_is_not_gzip_compressed() {
	let dir = tempfile::tempdir().expect("tempdir");
	let file = dir.path().join("data.txt");
	std::fs::write(&file, b"hello").expect("write");
	let bytes = pack_to_vec(&file, false, None, false);
	assert!(
		bytes.len() >= 2,
		"archive must have at least two bytes, got {}",
		bytes.len()
	);
	assert_ne!(
		&bytes[..2],
		&[0x1f, 0x8b],
		"archive must not start with the gzip magic header: first bytes are {:02x?}",
		&bytes[..bytes.len().min(8)]
	);
	// And it is a plain tar the destination can extract: one entry, named
	// after the source file.
	let mut archive = tar::Archive::new(bytes.as_slice());
	let names: Vec<String> = archive
		.entries()
		.expect("entries")
		.map(|e| {
			e.expect("entry")
				.path()
				.expect("path")
				.to_string_lossy()
				.into_owned()
		})
		.collect();
	assert_eq!(names, vec!["data.txt".to_string()]);
}

#[test]
fn pack_path_directory() {
	let dir = tempfile::tempdir().expect("tempdir");
	let subdir = dir.path().join("mydir");
	std::fs::create_dir(&subdir).expect("mkdir");
	std::fs::write(subdir.join("a.txt"), b"aaa").expect("write");
	std::fs::write(subdir.join("b.txt"), b"bbb").expect("write");
	let bytes = pack_to_vec(&subdir, false, None, false);
	assert!(!bytes.is_empty());
}

#[test]
fn pack_path_missing_source_is_a_cp_error() {
	// A missing host source on `cp` must read as a cp error, not a build error.
	// `src.is_dir()` returns false for a missing path, so `record_one` reaches
	// `symlink_metadata`, which fails on a missing file; the error mapper
	// (`cp_err`) wraps that as a `ComposeError::Copy`.
	let missing = Path::new("/nonexistent-host-source-xyz");
	let mut tar = tar::Builder::new(Vec::new());
	let mut sent = Vec::new();
	let err =
		super::super::pack_path(missing, false, None, false, &mut tar, &mut sent).unwrap_err();
	let msg = err.to_string();
	assert!(msg.contains("cp error"), "wrong category: {msg:?}");
	assert!(
		!msg.contains("build error"),
		"must not be a build error: {msg:?}"
	);
}

/// `cp host/. svc:/X` packs the directory's children at the top of the archive
/// instead of under the directory's name. There is no `payload/` wrapper
/// entry: the names that come back are the directory's children, exactly.
#[test]
fn pack_path_contents_packs_children_at_the_top_with_no_wrapper() {
	let dir = tempfile::tempdir().expect("tempdir");
	let payload = dir.path().join("payload");
	std::fs::create_dir(&payload).expect("mkdir");
	std::fs::write(payload.join("a.txt"), b"aaa").expect("write");
	std::fs::write(payload.join("b.txt"), b"bbb").expect("write");

	let bytes = pack_to_vec(&payload, false, None, true);

	// No wrapper, no `payload/` prefix: every name the archive lists is a
	// child of the source directory.
	let names = pack_path_entry_names(&bytes);
	assert_eq!(
		names,
		vec!["a.txt".to_string(), "b.txt".to_string()],
		"trailing `/.` packs the children at the top, got {names:?}"
	);
	assert!(
		!names.iter().any(|n| n == "payload"),
		"no wrapper entry named after the source dir, got {names:?}"
	);
}

/// Without the `/.` marker the wrapper is back: the same tree is now packed
/// under `payload/...`, the way `cp` has always packed a directory.
#[test]
fn pack_path_no_contents_keeps_the_wrapper() {
	let dir = tempfile::tempdir().expect("tempdir");
	let payload = dir.path().join("payload");
	std::fs::create_dir(&payload).expect("mkdir");
	std::fs::write(payload.join("a.txt"), b"aaa").expect("write");
	std::fs::write(payload.join("b.txt"), b"bbb").expect("write");

	let bytes = pack_to_vec(&payload, false, None, false);

	let names = pack_path_entry_names(&bytes);
	assert!(
		names.contains(&"payload".to_string()),
		"the wrapper entry must be present without the trailing `/.`, got {names:?}"
	);
	assert!(
		names.contains(&"a.txt".to_string()) && names.contains(&"b.txt".to_string()),
		"the children must still be in the archive, got {names:?}"
	);
}

/// `cp single-file/. svc:/X` is an error: the `/.` marker demands a
/// directory. Matches the "not a directory" shape `podman cp` returns.
#[test]
fn pack_path_contents_on_a_single_file_is_an_error() {
	let dir = tempfile::tempdir().expect("tempdir");
	let file = dir.path().join("single.txt");
	std::fs::write(&file, b"hi").expect("write");

	let mut tar = tar::Builder::new(Vec::new());
	let mut sent = Vec::new();
	let err = super::super::pack_path(&file, false, None, true, &mut tar, &mut sent).unwrap_err();
	let msg = err.to_string();
	assert!(
		msg.contains("not a directory"),
		"single-file source with `/.` must error as not a directory, got {msg:?}"
	);
}

/// The recorded list equals what `sent_entries` reads back from the same
/// bytes. Pack to a `Vec` with the recorder on, then assert
/// `recorded == sent_entries(&vec)`. Pins that the recorder and the archive
/// bytes agree on every entry — a "tar.append_link without recording"
/// shape breaks the parity, and any future drift on the other side breaks
/// this too.
#[cfg(unix)]
#[test]
fn the_recorder_agrees_with_what_sent_entries_reads_back() {
	let dir = tempfile::tempdir().expect("tempdir");
	let payload = dir.path().join("payload");
	std::fs::create_dir_all(payload.join("sub")).unwrap();
	std::fs::create_dir(payload.join("empty")).unwrap();
	std::fs::write(payload.join("plain.txt"), b"beside-the-rest").unwrap();
	std::fs::write(payload.join("sub/inner.bin"), vec![7u8; 1234]).unwrap();
	std::fs::write(payload.join("sub/nothing"), b"").unwrap();
	std::os::unix::fs::symlink("nowhere", payload.join("dangling")).unwrap();

	let (bytes, recorded) = pack_with_recorder(&payload, false, None, false);
	let from_archive = super::super::super::verify::sent_entries(&bytes).expect("sent_entries");

	let mut a: Vec<(String, String)> = recorded
		.iter()
		.map(|e| (e.path.clone(), format!("{:?}", e.kind)))
		.collect();
	let mut b: Vec<(String, String)> = from_archive
		.iter()
		.map(|e| (e.path.clone(), format!("{:?}", e.kind)))
		.collect();
	a.sort();
	b.sort();
	assert_eq!(
		a, b,
		"the recorder must equal sent_entries of the same bytes"
	);
}
