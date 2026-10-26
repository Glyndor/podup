use super::*;

/// P1. `R/sub/../link/out`: the parent is spelled `R/sub/..` but resolves
/// to `R` physically, so it is the same trusted link as `R/link/out`.
#[cfg(unix)]
#[test]
fn a_trusted_root_link_with_dotdot_at_an_intermediate_parent_is_let_through() {
	let (_dir, base) = tree_for_path_spelling();
	let dst = base.join("sub/../link/out");
	let trusted = trusted_for(&base);
	assert!(
		destination_refusal_under(&dst, Some(trusted)).is_none(),
		"R/sub/../link/out: the parent R/sub/.. resolves to R and the link must be trusted",
	);
}

/// P2. A relative path to `R/link/out` built with the existing
/// `relative_to_cwd` helper: 64 leading `..`s land the walk at `/` no
/// matter the CWD, so the symlink at the end is reached as a relative
/// path and its parent canonicalises to the trusted root.
#[cfg(unix)]
#[test]
fn a_relative_path_through_a_trusted_root_link_is_let_through() {
	let (_dir, base) = tree_for_path_spelling();
	let dst = relative_to_cwd(&base.join("link/out"));
	let trusted = trusted_for(&base);
	assert!(
		destination_refusal_under(&dst, Some(trusted)).is_none(),
		"relative path to R/link/out: the parent canonicalises to R and the link must be trusted",
	);
}

/// P3. `R/sub/../sub/inner_link/out` where `R/sub/inner_link` is a symlink
/// one level down: its parent is `R/sub`, not `R`, and the walk refuses it
/// even though the spelling goes through `..`. The `..` does not bring the
/// walk back to `R` because the link itself sits one level below.
#[cfg(unix)]
#[test]
fn a_trusted_root_check_via_dotdot_still_refuses_a_link_one_level_down() {
	let (_dir, base) = tree_for_path_spelling();
	std::os::unix::fs::symlink(base.join("real/elsewhere"), base.join("sub/inner_link"))
		.expect("symlink sub/inner_link");
	let dst = base.join("sub/../sub/inner_link/out");
	let trusted = trusted_for(&base);
	let err = destination_refusal_under(&dst, Some(trusted))
		.expect("R/sub/inner_link is one level below R and must be refused");
	let named = format!(
		"{} is a symlink",
		base.join("sub/../sub/inner_link").display()
	);
	assert!(
		err.to_string().contains(&named),
		"R/sub/../sub/inner_link/out: refusal must name R/sub/../sub/inner_link, got: {err}",
	);
}

/// P4. `R/real/../link/out`: the parent `R/real/..` is `R` physically, so
/// this is the same trusted link as `R/link/out` and must be let through.
#[cfg(unix)]
#[test]
fn a_trusted_root_link_with_dotdot_at_a_real_parent_is_let_through() {
	let (_dir, base) = tree_for_path_spelling();
	let dst = base.join("real/../link/out");
	let trusted = trusted_for(&base);
	assert!(
		destination_refusal_under(&dst, Some(trusted)).is_none(),
		"R/real/../link/out: the parent R/real/.. resolves to R and the link must be trusted",
	);
}
