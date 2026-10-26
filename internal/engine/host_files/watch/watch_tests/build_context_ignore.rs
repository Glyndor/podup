use super::*;

// --- build-context ignore ---------------------------------------------------

#[test]
fn build_context_ignore_patterns_load_once_and_match_against_context_relative() {
	let rule = Path::new("/proj/api");
	let ctx = Some(Path::new("/proj"));
	// `*.log` lives in the build context's ignore file, so it must match
	// the path relative to the build context (`api/server.log`), not
	// relative to the rule's path (`server.log`).
	let (rule_rel, ctx_rel) = rel_paths(&PathBuf::from("/proj/api/server.log"), rule, ctx);
	assert_eq!(rule_rel, "server.log");
	assert_eq!(ctx_rel.as_deref(), Some("api/server.log"));
	let ctx_patterns = pats(&["*.log"]);
	let d = evaluate_ignore(&[], &ctx_patterns, ctx_rel.as_deref(), &rule_rel);
	assert_eq!(d, Some(true));
	// `*.log` against the rule-relative path alone would be a substring
	// match, not a `.log` extension match; the build-context-aware
	// evaluator routes the pattern to the build-context-relative path so
	// `server.log` does not get ignored by accident.
	let d = evaluate_ignore(&[], &ctx_patterns, None, &rule_rel);
	assert_eq!(d, None);
}

#[test]
fn build_context_and_rule_patterns_combine_with_last_match_wins() {
	// Build-context `*.log` ignores `api/server.log` (true). The rule's
	// own `!server.log` then re-includes it (false). Last match wins.
	let rule = Path::new("/proj/api");
	let ctx = Some(Path::new("/proj"));
	let (rule_rel, ctx_rel) = rel_paths(&PathBuf::from("/proj/api/server.log"), rule, ctx);
	let ctx_patterns = pats(&["*.log"]);
	let rule_patterns = pats(&["!server.log"]);
	let d = evaluate_ignore(&rule_patterns, &ctx_patterns, ctx_rel.as_deref(), &rule_rel);
	assert_eq!(d, Some(false));
}
