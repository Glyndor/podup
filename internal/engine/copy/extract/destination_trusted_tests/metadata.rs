use super::*;

/// L1. A trusted root link whose target is a directory: the gates see the
/// directory behind the link, not the link itself.
#[cfg(unix)]
#[test]
fn metadata_follows_a_trusted_root_link_to_a_directory() {
	let (_dir, base) = tree_for_metadata();
	let trusted = trusted_for(&base);
	let meta = destination_metadata_under(&base.join("link"), Some(trusted)).expect("trusted link");
	assert!(
		meta.file_type().is_dir(),
		"R/link under valid policy: result must be a directory, got {:?}",
		meta.file_type(),
	);
	assert!(
		!meta.file_type().is_symlink(),
		"R/link under valid policy: result must NOT be a symlink",
	);
}

/// L2. Same destination, `trusted = None` (the non-Unix contract): the
/// gates see the symlink.
#[cfg(unix)]
#[test]
fn metadata_keeps_a_trusted_root_link_as_a_symlink_when_no_policy_is_passed() {
	let (_dir, base) = tree_for_metadata();
	let meta = destination_metadata_under(&base.join("link"), None).expect("symlink_metadata");
	assert!(
		meta.file_type().is_symlink(),
		"R/link under trusted = None: result must be a symlink, got {:?}",
		meta.file_type(),
	);
}

/// L3a. Same destination, `root_uid = own`, `link_uid = own + 1`: the link
/// is not owned by `link_uid`, so the policy does not apply and the gates
/// see the symlink. Flipped from L3b to exercise the link owner check
/// alone.
#[cfg(unix)]
#[test]
fn metadata_keeps_a_root_link_as_a_symlink_when_the_link_uid_does_not_match() {
	let (_dir, base) = tree_for_metadata();
	let meta = std::fs::metadata(&base).expect("metadata");
	let trusted = TrustedRoot {
		root: &base,
		root_uid: meta.uid(),
		link_uid: meta.uid() + 1,
	};
	let meta =
		destination_metadata_under(&base.join("link"), Some(trusted)).expect("symlink_metadata");
	assert!(
		meta.file_type().is_symlink(),
		"R/link under wrong link_uid: result must be a symlink, got {:?}",
		meta.file_type(),
	);
}

/// L3b. Same destination, `root_uid = own + 1`, `link_uid = own`: the root
/// is not owned by `root_uid`, so the policy does not apply and the gates
/// see the symlink. Flipped from L3a to exercise the root owner check
/// alone.
#[cfg(unix)]
#[test]
fn metadata_keeps_a_root_link_as_a_symlink_when_the_root_uid_does_not_match() {
	let (_dir, base) = tree_for_metadata();
	let meta = std::fs::metadata(&base).expect("metadata");
	let trusted = TrustedRoot {
		root: &base,
		root_uid: meta.uid() + 1,
		link_uid: meta.uid(),
	};
	let meta =
		destination_metadata_under(&base.join("link"), Some(trusted)).expect("symlink_metadata");
	assert!(
		meta.file_type().is_symlink(),
		"R/link under wrong root_uid: result must be a symlink, got {:?}",
		meta.file_type(),
	);
}

/// L4. A link one level below the trusted root is NOT a root link, so the
/// policy does not apply and the gates see the symlink.
#[cfg(unix)]
#[test]
fn metadata_keeps_a_below_root_link_as_a_symlink() {
	let (_dir, base) = tree_for_metadata();
	let trusted = trusted_for(&base);
	let meta = destination_metadata_under(&base.join("dir/link"), Some(trusted))
		.expect("symlink_metadata");
	assert!(
		meta.file_type().is_symlink(),
		"R/dir/link under valid policy: result must be a symlink, got {:?}",
		meta.file_type(),
	);
}

/// L5. A trailing separator must not let the kernel follow the link: the
/// function rebuilds `dst` from `components()`, which drops the separator,
/// so `lstat(R/link)` is still the symlink.
#[cfg(unix)]
#[test]
fn metadata_keeps_a_trailing_separator_from_making_the_kernel_follow_the_link() {
	let (_dir, base) = tree_for_metadata();
	let mut dst = base.join("link").into_os_string();
	dst.push(std::path::MAIN_SEPARATOR_STR);
	let meta = destination_metadata_under(Path::new(&dst), None).expect("symlink_metadata");
	assert!(
		meta.file_type().is_symlink(),
		"R/link/ under trusted = None: result must be a symlink (trailing separator must not follow), got {:?}",
		meta.file_type(),
	);
}

/// L6. A trusted root link whose target does not exist: following it
/// returns `NotFound`. Both gates already treat any error as "not an
/// existing directory", so this is the documented behaviour.
#[cfg(unix)]
#[test]
fn metadata_returns_not_found_for_a_dangling_trusted_link() {
	let (_dir, base) = tree_for_metadata();
	let trusted = trusted_for(&base);
	let err = destination_metadata_under(&base.join("dangling"), Some(trusted))
		.expect_err("dangling trusted link: follow must fail");
	assert_eq!(
		err.kind(),
		std::io::ErrorKind::NotFound,
		"R/dangling under valid policy: expected NotFound, got {err}",
	);
}

/// L7. A plain directory and a plain file, any policy: the helper does
/// not introduce its own behaviour for non-symlinks, so it returns the
/// same metadata `std::fs::symlink_metadata` would.
#[cfg(unix)]
#[test]
fn metadata_agrees_with_symlink_metadata_on_a_plain_directory_and_a_plain_file() {
	let (_dir, base) = tree_for_metadata();
	let plain_dir = base.join("real");
	let plain_file = base.join("real/file");
	for path in [&plain_dir, &plain_file] {
		let direct = std::fs::symlink_metadata(path).expect("symlink_metadata");
		let via_helper = destination_metadata_under(path, Some(trusted_for(&base)))
			.expect("destination_metadata_under");
		assert_eq!(
			direct.len(),
			via_helper.len(),
			"{}: size mismatch (direct {} vs helper {})",
			path.display(),
			direct.len(),
			via_helper.len(),
		);
		assert_eq!(
			direct.file_type(),
			via_helper.file_type(),
			"{}: file_type mismatch (direct {:?} vs helper {:?})",
			path.display(),
			direct.file_type(),
			via_helper.file_type(),
		);
		assert_eq!(
			direct.is_dir(),
			via_helper.is_dir(),
			"{}: is_dir mismatch",
			path.display(),
		);
	}
}
