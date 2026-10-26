use super::*;

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
