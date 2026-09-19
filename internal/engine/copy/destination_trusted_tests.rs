//! Trusted-root link tests for `destination.rs`.
//!
//! T1..T8 exercise the walk with a temp directory `R` as the trusted root and
//! the test process's own uid as the trusted uid, so the exception can be
//! let through, refused, narrowed and bypassed without running as root.
//! L1..L7 exercise `destination_metadata_under`, the helper the gates
//! (`cp_destination_kind`, `extract_archive`) call where they used to call
//! `std::fs::symlink_metadata(dst)` directly: a trusted link whose target
//! the operator typed IS the destination must be followed, every other
//! link still has to read as a link.

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;

#[cfg(unix)]
use super::destination::{
	destination_metadata_under, destination_refusal, destination_refusal_under, TrustedRoot,
};

// -- T1..T8: the trusted-root exception is a thin hole, not a tunnel. -----
//
// The tests in this section exercise `destination_refusal_under` with a temp
// directory `R` as the trusted root and the test process's own uid as the
// trusted uid, so the exception can be let through, refused, narrowed and
// bypassed without root. L1..L7 live further down and exercise the helper the
// gates use to inspect the destination after the walk accepted it.

/// Tree used by T1..T7. The links are placed so the relevant test outcomes
/// are read off the lstat walk directly:
///
/// ```text
/// R/real               regular dir
/// R/real/sub           regular dir
/// R/real/mid -> real/sub   symlink (the link the trusted walk bumps into)
/// R/link -> real          symlink at the root level (the trusted link)
/// R/dir                 regular dir
/// R/dir/link -> real/elsewhere   symlink one level below `R`
/// ```
#[cfg(unix)]
fn tree_with_root_links() -> (tempfile::TempDir, PathBuf) {
	let dir = tempfile::tempdir().expect("tempdir");
	let base = dir.path().canonicalize().expect("canonicalize");
	// `tempfile::tempdir` honors the process umask, so `R` could land as
	// 0o775 on a machine whose umask is 0o002. The trusted-root exception
	// requires `mode & 0o022 == 0`, so normalize once so T1, T2, T3, T4, T6
	// and T7 see a root that qualifies regardless of umask. T5 takes the
	// same tree and explicitly chmods R to 0o775 to exercise the refusal.
	std::fs::set_permissions(&base, PermissionsExt::from_mode(0o755)).expect("chmod 0o755");
	std::fs::create_dir_all(base.join("real")).expect("mkdir real");
	std::fs::create_dir_all(base.join("real/sub")).expect("mkdir real/sub");
	std::os::unix::fs::symlink(base.join("real/sub"), base.join("real/mid"))
		.expect("symlink real/mid");
	std::os::unix::fs::symlink(base.join("real"), base.join("link")).expect("symlink link");
	std::fs::create_dir_all(base.join("dir")).expect("mkdir dir");
	std::os::unix::fs::symlink(base.join("real/elsewhere"), base.join("dir/link"))
		.expect("symlink dir/link");
	(dir, base)
}

/// Build the trusted root the tests pass in: the temp directory `base` with
/// the calling process's own uid as both `root_uid` and `link_uid`, so the
/// link created under it is owned by the trusted uid and the root is too.
#[cfg(unix)]
fn trusted_for(base: &Path) -> TrustedRoot<'_> {
	let meta = std::fs::metadata(base).expect("metadata");
	TrustedRoot {
		root: base,
		root_uid: meta.uid(),
		link_uid: meta.uid(),
	}
}

/// The same location spelled relative to the working directory. `..` is
/// resolved physically, so more of them than the working directory is deep
/// always reach the root, and the test never has to change directory.
#[cfg(unix)]
fn relative_to_cwd(absolute: &Path) -> PathBuf {
	let mut out = PathBuf::new();
	for _ in 0..64 {
		out.push("..");
	}
	out.join(absolute.strip_prefix("/").expect("absolute"))
}

/// T1. A symlink at the first component under the trusted root is let
/// through when the link and the root are owned by the trusted uid and the
/// root has no group or other write bit.
#[cfg(unix)]
#[test]
fn a_trusted_root_link_is_let_through() {
	let (_dir, base) = tree_with_root_links();
	let dst = base.join("link/out");
	let trusted = trusted_for(&base);
	assert!(
		destination_refusal_under(&dst, Some(trusted)).is_none(),
		"R/link/out: a root-level trusted link must be let through",
	);
}

/// T2. The walk DOES NOT stop at a trusted root link: every component below
/// it is `lstat`ed through it, and a second link encountered on the way is
/// still refused. The refusal message names the second link as written, not
/// its target, because the path is walked as written.
#[cfg(unix)]
#[test]
fn the_walk_continues_below_a_trusted_root_link() {
	let (_dir, base) = tree_with_root_links();
	let dst = base.join("link/mid/leaf");
	let trusted = trusted_for(&base);
	let err = destination_refusal_under(&dst, Some(trusted)).expect(
		"R/link/mid must be refused: R/real/mid is a symlink one level below the trusted root link",
	);
	let named = format!("{} is a symlink", base.join("link/mid").display());
	assert!(
		err.to_string().contains(&named),
		"R/link/mid/leaf: the refusal must name the second link, got: {err}",
	);
}

/// T3. A link one level below the trusted root is NOT covered by the
/// exception. The first-component rule is exactly that: the first one.
#[cfg(unix)]
#[test]
fn a_link_one_level_below_the_trusted_root_is_refused() {
	let (_dir, base) = tree_with_root_links();
	let dst = base.join("dir/link/out");
	let trusted = trusted_for(&base);
	let err = destination_refusal_under(&dst, Some(trusted))
		.expect("R/dir/link must be refused: it sits one level below the trusted root");
	let named = format!("{} is a symlink", base.join("dir/link").display());
	assert!(
		err.to_string().contains(&named),
		"R/dir/link/out: the refusal must name R/dir/link, got: {err}",
	);
}

/// T4a. The exception trusts the link only when the link itself is owned by
/// `link_uid`. With the root owned by `root_uid` and the link owned by
/// someone else, the walk falls through to the normal refusal. The test
/// process cannot `chown`, so `link_uid = own + 1` is the only way to make
/// this check fail on its own; T4b below flips the two to fail the root
/// check instead.
#[cfg(unix)]
#[test]
fn a_trusted_root_link_not_owned_by_the_link_uid_is_refused() {
	let (_dir, base) = tree_with_root_links();
	let meta = std::fs::metadata(&base).expect("metadata");
	let trusted = TrustedRoot {
		root: &base,
		root_uid: meta.uid(),
		link_uid: meta.uid() + 1,
	};
	let dst = base.join("link/out");
	let err = destination_refusal_under(&dst, Some(trusted))
		.expect("link_uid mismatch: the link is not owned by trusted.link_uid and must be refused");
	let named = format!("{} is a symlink", base.join("link").display());
	assert!(
		err.to_string().contains(&named),
		"R/link/out under wrong link_uid: the refusal must name R/link, got: {err}",
	);
}

/// T4b. The exception trusts the root only when the root itself is owned by
/// `root_uid` and has no group or other write bit. With the link owned by
/// `link_uid` and the root owned by someone else, the walk still falls
/// through to the normal refusal. Flipped from T4a to exercise the root
/// owner check alone.
#[cfg(unix)]
#[test]
fn a_trusted_root_link_in_a_root_not_owned_by_the_root_uid_is_refused() {
	let (_dir, base) = tree_with_root_links();
	let meta = std::fs::metadata(&base).expect("metadata");
	let trusted = TrustedRoot {
		root: &base,
		root_uid: meta.uid() + 1,
		link_uid: meta.uid(),
	};
	let dst = base.join("link/out");
	let err = destination_refusal_under(&dst, Some(trusted))
		.expect("root_uid mismatch: the root is not owned by trusted.root_uid and must be refused");
	let named = format!("{} is a symlink", base.join("link").display());
	assert!(
		err.to_string().contains(&named),
		"R/link/out under wrong root_uid: the refusal must name R/link, got: {err}",
	);
}

/// T5. The exception trusts the root only when the root has no group or
/// other write bit (`mode & 0o022 == 0`). A group-writable root means
/// somebody other than the operator could plant a link there, so the link
/// is still refused. The mode is restored before the temp dir drops so
/// cleanup does not depend on the perm.
#[cfg(unix)]
#[test]
fn a_trusted_root_link_in_a_group_writable_root_is_refused() {
	let (_dir, base) = tree_with_root_links();
	std::fs::set_permissions(&base, PermissionsExt::from_mode(0o775)).expect("chmod 0o775");
	let trusted = trusted_for(&base);
	let dst = base.join("link/out");
	let result = destination_refusal_under(&dst, Some(trusted));
	std::fs::set_permissions(&base, PermissionsExt::from_mode(0o755))
		.expect("restore chmod to 0o755");
	let err = result.expect("group-writable root: the link must be refused");
	let named = format!("{} is a symlink", base.join("link").display());
	assert!(
		err.to_string().contains(&named),
		"R/link/out under group-writable root: refusal must name R/link, got: {err}",
	);
}

/// T6. The non-Unix contract is `trusted = None`: the exception does not
/// exist and every symlink is refused. Production's `destination_refusal`
/// passes `None` on non-Unix; on Unix we pass `None` here to simulate that.
#[cfg(unix)]
#[test]
fn no_trusted_root_refuses_every_symlink() {
	let (_dir, base) = tree_with_root_links();
	let dst = base.join("link/out");
	let err = destination_refusal_under(&dst, None)
		.expect("trusted = None is the non-Unix contract: every symlink refused");
	let named = format!("{} is a symlink", base.join("link").display());
	assert!(
		err.to_string().contains(&named),
		"trusted = None: refusal must name R/link, got: {err}",
	);
}

/// T7. The numbers matter: a guard that accepted everything, or refused
/// everything, would satisfy any single case above. A single table
/// asserting how many destinations in the accepted set were actually
/// accepted and how many in the refused set were actually refused catches
/// both regressions at once.
#[cfg(unix)]
#[test]
fn the_trusted_root_exception_accepts_exactly_the_accepted_set() {
	let (_dir, base) = tree_with_root_links();
	let accepted: Vec<(&str, PathBuf)> = vec![
		("trusted root link then missing leaf", base.join("link/out")),
		("symlink-free real dir", base.join("real/sub")),
	];
	let refused: Vec<(&str, PathBuf)> = vec![
		(
			"trusted root link then intermediate link",
			base.join("link/mid/leaf"),
		),
		(
			"intermediate link at the same depth",
			base.join("real/mid/leaf"),
		),
		(
			"link one level below the trusted root",
			base.join("dir/link/out"),
		),
	];
	let accepted_count = accepted.len();
	let refused_count = refused.len();
	let actually_accepted: Vec<&str> = accepted
		.iter()
		.filter(|(_, p)| destination_refusal_under(p, Some(trusted_for(&base))).is_none())
		.map(|(l, _)| *l)
		.collect();
	let actually_refused: Vec<&str> = refused
		.iter()
		.filter(|(_, p)| destination_refusal_under(p, Some(trusted_for(&base))).is_some())
		.map(|(l, _)| *l)
		.collect();
	assert_eq!(
		actually_accepted.len(),
		accepted_count,
		"destinations that should have been accepted but were refused: {actually_refused:?}",
	);
	assert_eq!(
		actually_refused.len(),
		refused_count,
		"destinations that should have been refused but were accepted: {actually_accepted:?}",
	);
}

/// T8. The exception only applies to prefixes whose parent IS the trusted
/// root. A relative destination has no root component, so the exception
/// can never apply, and a link encountered in the walk is still refused.
/// `relative_to_cwd` from above is reused: 64 leading `..`s land the walk
/// at `/` no matter the CWD.
#[cfg(unix)]
#[test]
fn a_relative_destination_through_a_link_is_refused_even_with_the_exception() {
	let dir = tempfile::tempdir().expect("tempdir");
	let base = dir.path().canonicalize().expect("canonicalize");
	std::fs::create_dir_all(base.join("real")).expect("mkdir real");
	std::os::unix::fs::symlink(base.join("real/victim"), base.join("real/link"))
		.expect("symlink real/link");
	let dst = relative_to_cwd(&base.join("real/link/out"));
	let err = destination_refusal(&dst).expect(
		"a relative destination through a symlink must be refused regardless of the exception",
	);
	let named = format!("{} is a symlink", base.join("real/link").display());
	assert!(
		err.to_string().contains(&named),
		"relative destination through a symlink must name the link component: got {err}",
	);
}

// -- L1..L7: the trusted-link helper the gates call after the walk. ------
//
// `destination_metadata_under(dst, trusted)` is the same call the two
// gates make after the walk accepted `dst`. It `lstat`s the destination
// itself (so an untrusted link still reads as a link), and follows it only
// when the walk would have let it through: `dst` is a trusted root link.

/// Tree for L1..L7. Smaller than `tree_with_root_links` because the metadata
/// helper only looks at the destination's last component and the trust
/// rule, not at intermediate links:
///
/// ```text
/// R/real               regular dir
/// R/real/file         regular file
/// R/link -> real          symlink at the root level (the trusted link)
/// R/dir                 regular dir
/// R/dir/link -> real     symlink one level below `R`
/// R/dangling -> nowhere  dangling root-level symlink (target does not exist)
/// ```
#[cfg(unix)]
fn tree_for_metadata() -> (tempfile::TempDir, PathBuf) {
	let dir = tempfile::tempdir().expect("tempdir");
	let base = dir.path().canonicalize().expect("canonicalize");
	std::fs::set_permissions(&base, PermissionsExt::from_mode(0o755)).expect("chmod 0o755");
	std::fs::create_dir_all(base.join("real")).expect("mkdir real");
	std::fs::write(base.join("real/file"), b"x").expect("write real/file");
	std::os::unix::fs::symlink(base.join("real"), base.join("link")).expect("symlink link");
	std::fs::create_dir_all(base.join("dir")).expect("mkdir dir");
	std::os::unix::fs::symlink(base.join("real"), base.join("dir/link")).expect("symlink dir/link");
	std::os::unix::fs::symlink(base.join("nowhere"), base.join("dangling"))
		.expect("symlink dangling");
	(dir, base)
}

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
