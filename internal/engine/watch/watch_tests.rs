//! Watch spec semantics: ignore / include evaluated against the path
//! relative to the rule's `path`, with the build context's ignore file
//! loaded as implicit `ignore` content and a legacy project-relative
//! fallback for rules written before this change.
//!
//! These unit tests exercise the per-rule evaluator directly, not the
//! full watch loop with the notify watcher; the loop integration is
//! pinned by the live test in `tests/engine_integration/watch.rs`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::ignore_filter::{
	evaluate_ignore, ignored_with_fallback, included_with_fallback, legacy_ignore_fallback,
	RuleContext,
};
use super::mod_test_hooks::legacy_pattern_suggestion;
use super::sync::{legacy_project_relative_ignored, legacy_project_relative_included};

fn pats(v: &[&str]) -> Vec<String> {
	v.iter().map(|s| s.to_string()).collect()
}

/// Compute the rule-relative and (optional) build-context-relative path for
/// `full` under `rule` and `ctx`. Mirrors what the watch loop does at
/// dispatch time.
fn rel_paths(full: &Path, rule: &Path, ctx: Option<&Path>) -> (String, Option<String>) {
	let rule_rel = full
		.strip_prefix(rule)
		.unwrap_or(full)
		.to_string_lossy()
		.replace('\\', "/");
	let ctx_rel = ctx
		.and_then(|root| full.strip_prefix(root).ok())
		.map(|p| p.to_string_lossy().replace('\\', "/"));
	(rule_rel, ctx_rel)
}

/// Build a `RuleContext` for a one-shot test invocation.
fn make_ctx<'a>(
	rule_abs: &'a Path,
	base_dir: &'a Path,
	service_name: &'a str,
	rule_path: &'a str,
	ctx_patterns: &'a [String],
	ctx_abs: Option<&'a Path>,
	warned: &'a mut HashSet<(String, String)>,
) -> RuleContext<'a> {
	RuleContext {
		rule_abs,
		ctx_abs,
		base_dir,
		service_name,
		rule_path,
		ctx_patterns,
		warned,
	}
}

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
	let d = crate::engine::ignore_patterns::evaluate(&rule_rel, &patterns);
	assert_eq!(d, Some(true));
	let (rule_rel, _ctx_rel) = rel_paths(&PathBuf::from("/proj/api/README.md"), rule, ctx);
	let d = crate::engine::ignore_patterns::evaluate(&rule_rel, &patterns);
	assert_eq!(d, None);
}

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

// --- per-rule pipeline (rule-relative strip + spec match + fallback) --------

#[test]
fn pipeline_ignores_when_rule_relative_pattern_matches() {
	// `path: ./api`, `ignore: [cache/]`, changed file `./api/cache/x`.
	// The pipeline must compute `cache/x` from the rule's path, evaluate
	// against `cache/`, and return `true`.
	let rule_abs = Path::new("/proj/api");
	let base = Path::new("/proj");
	let mut warned: HashSet<(String, String)> = HashSet::new();
	let mut ctx = make_ctx(rule_abs, base, "web", "./api", &[], None, &mut warned);
	let ignored = ignored_with_fallback(
		&PathBuf::from("/proj/api/cache/x"),
		&pats(&["cache/"]),
		&mut ctx,
	);
	assert!(
		ignored,
		"rule-relative `cache/` must ignore `./api/cache/x`"
	);
}

#[test]
fn pipeline_does_not_ignore_when_rule_relative_pattern_does_not_match() {
	let rule_abs = Path::new("/proj/api");
	let base = Path::new("/proj");
	let mut warned: HashSet<(String, String)> = HashSet::new();
	let mut ctx = make_ctx(rule_abs, base, "web", "./api", &[], None, &mut warned);
	let ignored = ignored_with_fallback(
		&PathBuf::from("/proj/api/src/main.rs"),
		&pats(&["cache/"]),
		&mut ctx,
	);
	assert!(
		!ignored,
		"`src/main.rs` is not under `cache/`, must not be ignored"
	);
}

#[test]
fn pipeline_ignores_via_project_relative_legacy_pattern_with_warning() {
	// `path: ./api`, `ignore: [api/cache]` (legacy form). The spec matcher
	// on `cache/x` says "no pattern matched"; the legacy matcher on
	// `./api/cache/x` says "matched", so the file is ignored and one
	// warning is recorded.
	let rule_abs = Path::new("/proj/api");
	let base = Path::new("/proj");
	let mut warned: HashSet<(String, String)> = HashSet::new();
	let mut ctx = make_ctx(rule_abs, base, "web", "./api", &[], None, &mut warned);
	let ignored = ignored_with_fallback(
		&PathBuf::from("/proj/api/cache/x"),
		&pats(&["api/cache"]),
		&mut ctx,
	);
	assert!(ignored);
	assert_eq!(
		warned.len(),
		1,
		"exactly one warning recorded for the legacy pattern"
	);
}

#[test]
fn pipeline_emits_one_warning_across_three_matching_events() {
	// The same (service, pattern) appears three times across the session
	// for matching events; the warning set records exactly one entry.
	let rule_abs = Path::new("/proj/api");
	let base = Path::new("/proj");
	let mut warned: HashSet<(String, String)> = HashSet::new();
	for _ in 0..3 {
		let mut ctx = make_ctx(rule_abs, base, "web", "./api", &[], None, &mut warned);
		let _ = ignored_with_fallback(
			&PathBuf::from("/proj/api/cache/x"),
			&pats(&["api/cache"]),
			&mut ctx,
		);
	}
	assert_eq!(warned.len(), 1);
}

#[test]
fn pipeline_negation_does_not_warn_when_spec_says_reinclude() {
	// `[cache/, !cache/keep]` against `./api/cache/keep`. The spec matcher
	// returns Some(false); the fallback never runs; no warning is recorded.
	let rule_abs = Path::new("/proj/api");
	let base = Path::new("/proj");
	let mut warned: HashSet<(String, String)> = HashSet::new();
	let mut ctx = make_ctx(rule_abs, base, "web", "./api", &[], None, &mut warned);
	let ignored = ignored_with_fallback(
		&PathBuf::from("/proj/api/cache/keep"),
		&pats(&["cache/", "!cache/keep"]),
		&mut ctx,
	);
	assert!(!ignored, "cache/keep is re-included by `!cache/keep`");
	assert!(warned.is_empty(), "no warning for spec-driven re-include");
}

#[test]
fn pipeline_negation_authoritative_against_legacy_matcher() {
	// The legacy matcher is anchored at the start of the project-relative
	// path. A user who wrote `api/cache/` (project-relative) into the
	// `ignore` list gets the spec matcher to say "no pattern matched"
	// (rule-relative is `cache/keep`, which does not match `api/cache/`);
	// the spec then says Some(false) because the negation `!cache/keep`
	// re-includes the file. The legacy matcher, run on the project-
	// relative `api/cache/keep`, would say `api/cache/` matches -> ignored.
	// Without the "nothing matched" guard the legacy decision wins and
	// the negation is silently overridden.
	let rule_abs = Path::new("/proj/api");
	let base = Path::new("/proj");
	let mut warned: HashSet<(String, String)> = HashSet::new();
	let mut ctx = make_ctx(rule_abs, base, "web", "./api", &[], None, &mut warned);
	let ignored = ignored_with_fallback(
		&PathBuf::from("/proj/api/cache/keep"),
		&pats(&["api/cache/", "!cache/keep"]),
		&mut ctx,
	);
	assert!(
		!ignored,
		"negation `!cache/keep` is authoritative: a legacy `api/cache/` pattern must not ignore `proj/api/cache/keep`"
	);
}

#[test]
fn pipeline_uses_build_context_patterns_when_present() {
	// Service has a local `build: .` (build context = project root) and
	// `.dockerignore` carries `*.log`. Changed file `./api/server.log`
	// should be ignored because the build-context pattern matches the
	// build-context-relative path (`api/server.log`).
	let rule_abs = Path::new("/proj/api");
	let base = Path::new("/proj");
	let ctx_abs = Some(Path::new("/proj"));
	let ctx_patterns = pats(&["*.log"]);
	let mut warned: HashSet<(String, String)> = HashSet::new();
	let mut ctx = make_ctx(
		rule_abs,
		base,
		"web",
		"./api",
		&ctx_patterns,
		ctx_abs,
		&mut warned,
	);
	let ignored = ignored_with_fallback(&PathBuf::from("/proj/api/server.log"), &[], &mut ctx);
	assert!(
		ignored,
		"build-context `*.log` must ignore `api/server.log` via the build-context-relative path"
	);
}

#[test]
fn pipeline_does_not_load_build_context_patterns_when_no_build() {
	// Same change as above, but the service has no `build:` (image-based);
	// no `.dockerignore` is loaded, so `server.log` reaches the container.
	let rule_abs = Path::new("/proj/api");
	let base = Path::new("/proj");
	let ctx_abs: Option<&Path> = None;
	let mut warned: HashSet<(String, String)> = HashSet::new();
	let mut ctx = make_ctx(rule_abs, base, "web", "./api", &[], ctx_abs, &mut warned);
	let ignored = ignored_with_fallback(&PathBuf::from("/proj/api/server.log"), &[], &mut ctx);
	assert!(
		!ignored,
		"no build context means no `.dockerignore` patterns are loaded"
	);
}

#[test]
fn pipeline_include_spec_matches_against_rule_relative_path() {
	let rule_abs = Path::new("/proj/api");
	let base = Path::new("/proj");
	let mut warned: HashSet<(String, String)> = HashSet::new();
	let mut ctx = make_ctx(rule_abs, base, "web", "./api", &[], None, &mut warned);
	let included = included_with_fallback(
		&PathBuf::from("/proj/api/src/main.rs"),
		&pats(&["*.rs"]),
		&mut ctx,
	);
	assert!(included);
	assert!(warned.is_empty());
}

#[test]
fn pipeline_include_old_form_trailing_segment_warns_once() {
	let rule_abs = Path::new("/proj/api");
	let base = Path::new("/proj");
	let mut warned: HashSet<(String, String)> = HashSet::new();
	let mut ctx = make_ctx(rule_abs, base, "web", "./api", &[], None, &mut warned);
	let included = included_with_fallback(
		&PathBuf::from("/proj/api/src/main.rs"),
		&pats(&["main.rs"]),
		&mut ctx,
	);
	assert!(
		included,
		"legacy `main.rs` still matches `./api/src/main.rs`"
	);
	assert_eq!(
		warned.len(),
		1,
		"exactly one warning for the legacy pattern"
	);
}

#[test]
fn pipeline_root_path_no_warning_for_matching_pattern() {
	// `path: .` makes the rule's abs path equal the project root, so the
	// rule-relative and project-relative paths coincide. The legacy matcher
	// would also match, but the watch loop never invokes it because the
	// spec matcher already decided. The pipeline therefore records no
	// warning.
	let rule_abs = Path::new("/proj");
	let base = Path::new("/proj");
	let mut warned: HashSet<(String, String)> = HashSet::new();
	let mut ctx = make_ctx(rule_abs, base, "web", ".", &[], None, &mut warned);
	let ignored = ignored_with_fallback(
		&PathBuf::from("/proj/cache/x"),
		&pats(&["cache/"]),
		&mut ctx,
	);
	assert!(ignored);
	assert!(
		warned.is_empty(),
		"path: . must not warn for a matching pattern"
	);
}

// --- legacy (project-relative) fallback ------------------------------------

#[test]
fn legacy_ignore_pattern_anchored_at_start_of_project_relative_path() {
	let p = pats(&["api/cache"]);
	assert!(legacy_project_relative_ignored("api/cache/x", &p));
	assert!(legacy_project_relative_ignored("api/cache", &p));
	assert!(!legacy_project_relative_ignored("./api/cache/x", &p));
	assert!(!legacy_project_relative_ignored("src/main.rs", &p));
}

#[test]
fn legacy_include_pattern_matches_trailing_segment() {
	let p = pats(&["main.rs"]);
	assert!(legacy_project_relative_included("./api/src/main.rs", &p));
	assert!(!legacy_project_relative_included("./api/src/lib.rs", &p));
}

#[test]
fn legacy_pattern_suggestion_strips_rule_path_prefix_when_present() {
	// `pattern = "api/cache"`, `rule_path = "./api"` -> suggest `cache`.
	let s = legacy_pattern_suggestion("api/cache", "./api");
	assert!(s.contains("\"cache\""), "got: {s}");
}

#[test]
fn legacy_pattern_suggestion_mentions_rule_path_when_no_prefix() {
	// `pattern = "node_modules/"`, `rule_path = "./api"` -> mention the
	// rule path so the user knows where the pattern is now read against.
	let s = legacy_pattern_suggestion("node_modules/", "./api");
	assert!(s.contains("\"./api\""), "got: {s}");
}

#[test]
fn fallback_warns_once_per_service_pattern_pair() {
	// Drive the spec path with no rule-pattern match, then run the
	// fallback three times for the same (service, pattern) and assert
	// that the warning set only grows by one.
	let rule = Path::new("/proj/api");
	let ctx: Option<&Path> = None;
	let patterns = pats(&["api/cache"]);
	let mut warned: HashSet<(String, String)> = HashSet::new();
	let full_path = PathBuf::from("/proj/api/cache/x");
	let (rule_rel, ctx_rel) = rel_paths(&full_path, rule, ctx);
	assert_eq!(
		evaluate_ignore(&patterns, &[], ctx_rel.as_deref(), &rule_rel),
		None
	);
	for _ in 0..3 {
		let _ = legacy_ignore_fallback(
			&full_path,
			Path::new("/proj"),
			"web",
			"./api",
			&patterns,
			&mut warned,
		);
	}
	assert_eq!(
		warned.len(),
		1,
		"exactly one warning per (service, pattern)"
	);
}

// --- build-context ignore loading -------------------------------------------

#[test]
fn local_build_context_patterns_loads_dockerignore() {
	// End-to-end: the build context's `.dockerignore` must be loaded by
	// `local_build_context_patterns` and made available to the
	// per-event pipeline. A `.log` file under the rule's path is then
	// ignored by the build-context pattern.
	let tmp = tempfile::tempdir().unwrap();
	let build_dir = tmp.path();
	std::fs::write(build_dir.join(".dockerignore"), b"*.log\n").unwrap();
	let service_yaml = format!("services:\n  web:\n    build: {}\n", build_dir.display());
	let file = crate::compose::parse_str(&service_yaml).unwrap();
	let service = file.services.get("web").unwrap();
	let (ctx_abs, ctx_patterns) =
		super::ignore_filter::local_build_context_patterns(tmp.path(), service);
	assert!(
		ctx_abs.is_some(),
		"local build context must be resolved for a service with build: <local path>"
	);
	assert_eq!(
		ctx_patterns,
		vec!["*.log".to_string()],
		"patterns loaded from the build context's `.dockerignore`"
	);
}

#[test]
fn local_build_context_patterns_omits_for_image_only_service() {
	// A service with no `build:` block (image-based) must not pretend
	// to have a build context or load any `.dockerignore`.
	let yaml = "services:\n  web:\n    image: alpine:latest\n";
	let file = crate::compose::parse_str(yaml).unwrap();
	let service = file.services.get("web").unwrap();
	let (ctx_abs, ctx_patterns) =
		super::ignore_filter::local_build_context_patterns(std::path::Path::new("/tmp"), service);
	assert!(ctx_abs.is_none(), "image-only service has no build context");
	assert!(ctx_patterns.is_empty());
}
