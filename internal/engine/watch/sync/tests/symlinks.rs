use super::*;

// --- symlink-to-directory at the top level -------------------------------

/// A symlink that points at a directory outside the watched rule must be
/// stored as a link, not as the contents of its target. `is_dir` followed
/// the link and walked the target directory; the fix uses `symlink_metadata`
/// so the link shape is what reaches the container (#1985).
#[cfg(unix)]
#[test]
fn sync_tar_symlink_to_directory_outside_rule_packs_the_link_only() {
	let dir = tempdir().unwrap();
	let outside = dir.path().join("outside");
	fs::create_dir(&outside).unwrap();
	fs::write(outside.join("secret.txt"), b"do not leak this").unwrap();

	let rule = dir.path().join("rule");
	fs::create_dir(&rule).unwrap();
	let link = rule.join("link");
	std::os::unix::fs::symlink("../outside", &link).unwrap();

	let bytes = sync_to_gz(&link, Path::new("link")).unwrap();
	let entries = tar_entries(&bytes);
	assert_eq!(
		entries,
		vec![("link".into(), "link")],
		"expected exactly one link entry named `link`, got {entries:?}"
	);
	assert_eq!(
		tar_link_target(&bytes, "link").as_deref(),
		Some("../outside"),
		"link target must be preserved verbatim"
	);
	// Defence in depth: no file named `link/secret.txt` slipped through.
	let names = tar_entry_paths(&bytes);
	assert!(
		!names.iter().any(|n| n.starts_with("link/")),
		"the link must not be unpacked into its target, got {names:?}"
	);
}
