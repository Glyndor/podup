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
#[cfg(unix)]
use super::destination_tests::relative_to_cwd;

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
pub(super) fn tree_with_root_links() -> (tempfile::TempDir, PathBuf) {
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
pub(super) fn trusted_for(base: &Path) -> TrustedRoot<'_> {
	let meta = std::fs::metadata(base).expect("metadata");
	TrustedRoot {
		root: base,
		root_uid: meta.uid(),
		link_uid: meta.uid(),
	}
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
pub(super) fn tree_for_metadata() -> (tempfile::TempDir, PathBuf) {
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

// -- P1..P4: the trusted-root check must not depend on how the path is
// spelled. --------------------------------------------------------------
//
// The buggy form compared the link's parent to the trusted root as a string,
// so `R/sub/../link/out` (parent is `R/sub/..`), `R/real/../link/out` (parent
// is `R/real/..`), and a relative path with 64 leading `..`s (parent ends in
// `link`) were all refused even though they name the same qualifying link.
// The fix canonicalises the parent before comparing it to the canonicalised
// trusted root, so all three spellings reach the same trusted link.

/// Tree used by P1..P4. Matches the description in the task: a temp dir `R`
/// as the trusted root, the process's own uid for both owners, `R/link ->
/// R/real`, and a real directory `R/sub`. The mode is normalised to `0o755`
/// because `tempfile::tempdir` honors the process umask and the
/// trusted-root exception requires `mode & 0o022 == 0`.
#[cfg(unix)]
pub(super) fn tree_for_path_spelling() -> (tempfile::TempDir, PathBuf) {
	let dir = tempfile::tempdir().expect("tempdir");
	let base = dir.path().canonicalize().expect("canonicalize");
	std::fs::set_permissions(&base, PermissionsExt::from_mode(0o755)).expect("chmod 0o755");
	std::fs::create_dir_all(base.join("real")).expect("mkdir real");
	std::fs::create_dir_all(base.join("sub")).expect("mkdir sub");
	std::os::unix::fs::symlink(base.join("real"), base.join("link")).expect("symlink link");
	(dir, base)
}

mod metadata;
mod path_spelling;
mod root_policy;
