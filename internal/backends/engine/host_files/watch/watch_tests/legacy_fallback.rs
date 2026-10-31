use super::*;

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
