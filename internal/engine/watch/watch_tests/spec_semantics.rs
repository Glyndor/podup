use super::*;

// --- spec semantics (path relative to the rule's `path`) -------------------

#[test]
fn rule_relative_dir_pattern_ignores_descendant() {
	let rule = Path::new("/proj/api");
	let ctx: Option<&Path> = None;
	let (rule_rel, ctx_rel) = rel_paths(&PathBuf::from("/proj/api/cache/x"), rule, ctx);
	assert_eq!(rule_rel, "cache/x");
	assert!(matches!(
		evaluate_ignore(&pats(&["cache/"]), &[], ctx_rel.as_deref(), &rule_rel),
		Some(true)
	));
}

#[test]
fn rule_relative_glob_extension_ignores_descendant() {
	let rule = Path::new("/proj/api");
	let ctx: Option<&Path> = None;
	let (rule_rel, ctx_rel) = rel_paths(&PathBuf::from("/proj/api/a/b.txt"), rule, ctx);
	assert_eq!(rule_rel, "a/b.txt");
	assert!(matches!(
		evaluate_ignore(&pats(&["*.txt"]), &[], ctx_rel.as_deref(), &rule_rel),
		Some(true)
	));
	let (rule_rel, ctx_rel) = rel_paths(&PathBuf::from("/proj/api/src/main.rs"), rule, ctx);
	assert_eq!(rule_rel, "src/main.rs");
	assert_eq!(
		evaluate_ignore(&pats(&["*.txt"]), &[], ctx_rel.as_deref(), &rule_rel),
		None
	);
}

#[test]
fn rule_relative_double_star_anywhere_ignores_descendant() {
	let rule = Path::new("/proj/api");
	let ctx: Option<&Path> = None;
	let (rule_rel, ctx_rel) = rel_paths(&PathBuf::from("/proj/api/x/cache/y"), rule, ctx);
	assert!(matches!(
		evaluate_ignore(&pats(&["**/cache/**"]), &[], ctx_rel.as_deref(), &rule_rel),
		Some(true)
	));
}

#[test]
fn rule_relative_subtree_pattern_ignores_descendant() {
	let rule = Path::new("/proj/api");
	let ctx: Option<&Path> = None;
	let (rule_rel, ctx_rel) = rel_paths(&PathBuf::from("/proj/api/cache/y"), rule, ctx);
	assert!(matches!(
		evaluate_ignore(&pats(&["cache/**"]), &[], ctx_rel.as_deref(), &rule_rel),
		Some(true)
	));
}

#[test]
fn rule_relative_pattern_does_not_match_unrelated_paths() {
	let rule = Path::new("/proj/api");
	let ctx: Option<&Path> = None;
	let (rule_rel, ctx_rel) = rel_paths(&PathBuf::from("/proj/api/src/main.rs"), rule, ctx);
	assert_eq!(
		evaluate_ignore(&pats(&["cache/"]), &[], ctx_rel.as_deref(), &rule_rel),
		None
	);
}

#[test]
fn negation_reinclude_is_authoritative_across_fallback() {
	// `[cache/, !cache/keep]`: `cache/keep` is re-included by the spec
	// matcher (Some(false)). The fallback only runs when the spec matcher
	// returned None; the authoritative Some(false) means the file is
	// always kept, regardless of what the old matcher would say.
	let rule = Path::new("/proj/api");
	let ctx: Option<&Path> = None;
	let (keep_rel, keep_ctx) = rel_paths(&PathBuf::from("/proj/api/cache/keep"), rule, ctx);
	let patterns = pats(&["cache/", "!cache/keep"]);
	assert_eq!(
		evaluate_ignore(&patterns, &[], keep_ctx.as_deref(), &keep_rel),
		Some(false)
	);
	let (other_rel, other_ctx) = rel_paths(&PathBuf::from("/proj/api/cache/other"), rule, ctx);
	assert_eq!(
		evaluate_ignore(&patterns, &[], other_ctx.as_deref(), &other_rel),
		Some(true)
	);
}

#[test]
fn include_spec_matches_against_rule_relative_path() {
	let rule = Path::new("/proj/api");
	let ctx: Option<&Path> = None;
	let (rule_rel, _ctx_rel) = rel_paths(&PathBuf::from("/proj/api/src/main.rs"), rule, ctx);
	let patterns = pats(&["*.rs"]);
	let d = crate::engine::context::ignore_patterns::evaluate(&rule_rel, &patterns);
	assert_eq!(d, Some(true));
	let (rule_rel, _ctx_rel) = rel_paths(&PathBuf::from("/proj/api/README.md"), rule, ctx);
	let d = crate::engine::context::ignore_patterns::evaluate(&rule_rel, &patterns);
	assert_eq!(d, None);
}
