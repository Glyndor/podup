//! Watch spec semantics: ignore / include evaluated against the path
//! relative to the rule's `path`, with the build context's ignore file
//! loaded as implicit `ignore` content and a legacy project-relative
//! fallback for rules written before this change.
//!
//! These unit tests exercise the per-rule evaluator directly, not the
//! full watch loop with the notify watcher; the loop integration is
//! pinned by the live test in `tests/engine_integration/host_files/watch/watch.rs`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::ignore_filter::{
	evaluate_ignore, ignored_with_fallback, included_with_fallback, legacy_ignore_fallback,
	legacy_pattern_suggestion, RuleContext,
};
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

mod build_context_ignore;
mod context_loading;
mod legacy_fallback;
mod pipeline;
mod spec_semantics;
