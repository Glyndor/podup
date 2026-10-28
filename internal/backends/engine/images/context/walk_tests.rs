use super::{walk_dir, walk_dir_skipping};
use std::path::PathBuf;

#[cfg(unix)]
#[test]
fn replacing_a_classified_directory_with_a_symlink_is_rejected() {
	let dir = tempfile::tempdir().unwrap();
	let outside = tempfile::tempdir().unwrap();
	let child = dir.path().join("child");
	std::fs::create_dir(&child).unwrap();
	std::fs::write(outside.path().join("private"), b"OUTSIDE-MARKER").unwrap();
	let result = walk_dir_skipping(dir.path(), |path| {
		if path == child {
			std::fs::remove_dir(&child).unwrap();
			std::os::unix::fs::symlink(outside.path(), &child).unwrap();
		}
		false
	});
	assert!(result.is_err(), "a changed directory must not be followed");
}

#[cfg(unix)]
#[test]
fn packing_an_opened_file_never_reopens_a_replacement_symlink() {
	use std::io::Read;
	let dir = tempfile::tempdir().unwrap();
	let outside = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("file"), b"inside").unwrap();
	std::fs::write(outside.path().join("private"), b"OUTSIDE-MARKER").unwrap();
	let mut tar = crate::engine::context::tar_stream::builder(Vec::new());
	super::visit_dir_skipping(
		dir.path(),
		|_| false,
		|_| true,
		|path, entry| {
			if let super::Entry::File(mut file) = entry {
				std::fs::remove_file(path)?;
				std::os::unix::fs::symlink(outside.path().join("private"), path)?;
				tar.append_file("file", &mut file)?;
			}
			Ok(())
		},
	)
	.unwrap();
	let bytes = tar.into_inner().unwrap();
	let mut archive = tar::Archive::new(bytes.as_slice());
	let mut entries = archive.entries().unwrap();
	let mut entry = entries.next().unwrap().unwrap();
	let mut body = Vec::new();
	entry.read_to_end(&mut body).unwrap();
	assert_eq!(body, b"inside");
}

#[cfg(unix)]
#[test]
fn replacing_an_opened_directory_cannot_redirect_its_children() {
	use std::io::Read;
	let dir = tempfile::tempdir().unwrap();
	let outside = tempfile::tempdir().unwrap();
	let child = dir.path().join("child");
	std::fs::create_dir(&child).unwrap();
	std::fs::write(child.join("file"), b"inside").unwrap();
	std::fs::write(outside.path().join("file"), b"OUTSIDE-MARKER").unwrap();
	let mut contents = Vec::new();
	super::visit_dir_skipping(
		dir.path(),
		|_| false,
		|_| true,
		|path, entry| {
			match entry {
				super::Entry::Directory(_) => {
					std::fs::rename(path, dir.path().join("saved"))?;
					std::os::unix::fs::symlink(outside.path(), path)?;
				}
				super::Entry::File(mut file) => {
					file.read_to_end(&mut contents)?;
				}
				_ => {}
			}
			Ok(())
		},
	)
	.unwrap();
	assert_eq!(contents, b"inside");
}

#[test]
fn walk_dir_returns_every_entry_in_sorted_order() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::create_dir_all(dir.path().join("sub")).unwrap();
	std::fs::write(dir.path().join("a"), b"a").unwrap();
	std::fs::write(dir.path().join("sub/b"), b"b").unwrap();
	let got: Vec<PathBuf> = walk_dir(dir.path()).unwrap();
	let names: Vec<String> = got
		.iter()
		.map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
		.collect();
	assert_eq!(names, vec!["a", "sub", "b"]);
}

/// #1746 entry 6: the build-context walk used to enumerate every entry
/// under an ignored directory and rely on the per-file filter to
/// drop them. For a `target/` with a million object files (Rust,
/// Node, …) that read happens before the filter can save any of it;
/// the cost is borne by `walk::walk_dir` even though the result
/// never reaches the tar. The new walker lets the call site mark a
/// directory as `skip_dir` and avoid the descent. The test builds a
/// directory the ignore pattern drops wholesale, plus one the
/// pattern leaves intact, and asserts the count of walked paths is
/// the number the per-file filter would have left, not the total
/// file count.
#[test]
fn walk_dir_skipping_prunes_ignored_subtrees() {
	let dir = tempfile::tempdir().unwrap();
	// A 200-file `target/` the ignore pattern drops wholesale.
	let target = dir.path().join("target");
	std::fs::create_dir(&target).unwrap();
	for i in 0..200 {
		std::fs::write(target.join(format!("obj{i}")), b"x").unwrap();
	}
	// A 5-file `src/` the ignore pattern leaves intact.
	let src = dir.path().join("src");
	std::fs::create_dir(&src).unwrap();
	for i in 0..5 {
		std::fs::write(src.join(format!("file{i}")), b"y").unwrap();
	}
	// `walk_dir` (the unfiltered form) visits every entry. The
	// pre-fix call site used this and accepted the cost.
	let unfiltered = walk_dir(dir.path()).unwrap();
	let unfiltered_under_target = unfiltered
		.iter()
		.filter(|p| p.strip_prefix(&target).is_ok())
		.count();
	assert_eq!(
		unfiltered_under_target, 201,
		"baseline: walk_dir must visit every entry under target/ before the fix"
	);
	// `walk_dir_skipping` with a closure that says "target is ignored"
	// visits `target/` (the empty dir is still shipped) but none of
	// its 200 contents. The non-ignored side (`src/`) is unaffected.
	let target_abs = target.clone();
	let got = walk_dir_skipping(dir.path(), move |abs| abs == target_abs).unwrap();
	let visited_under_target = got
		.iter()
		.filter(|p| p.strip_prefix(&target).is_ok())
		.count();
	assert_eq!(
		visited_under_target, 1,
		"walk_dir_skipping must not descend into the skipped directory; \
		 got {got:?}"
	);
	// Sanity: the un-skipped subtree is fully walked.
	let visited_under_src = got.iter().filter(|p| p.strip_prefix(&src).is_ok()).count();
	assert_eq!(
		visited_under_src, 6,
		"src/ is not skipped, full descent expected"
	);
}
