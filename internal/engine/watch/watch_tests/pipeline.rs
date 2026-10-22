use super::*;

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
