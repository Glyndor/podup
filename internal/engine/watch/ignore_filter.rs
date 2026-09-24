//! Per-rule ignore / include evaluation for `develop.watch` rules.
//!
//! The watch loop calls into here for every event path so each rule's spec
//! semantics live in one place: paths are read relative to the rule's
//! `path`, the build context's `.dockerignore` is loaded as implicit
//! `ignore` content, and a `!` re-include is authoritative. When the spec
//! matcher has nothing to say, a project-relative fallback keeps rules
//! written before this change working, with a one-shot warning per
//! distinct pattern that decides.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tracing::warn;

use crate::engine::build::is_remote_context;
use crate::engine::ignore_patterns::{evaluate, read_patterns};

use super::sync::{legacy_project_relative_ignored, legacy_project_relative_included};

/// Resolve the local build context's ignore file (if any) for `service`.
///
/// Returns `(absolute_path, patterns)`. `absolute_path` is the directory the
/// ignore file was looked up under, so the per-event evaluation can strip the
/// changed path against it; `patterns` is empty when the service has no local
/// `build:` (image-only or remote context) or when the ignore file is missing.
/// A remote context (URL/git) never has a local `.dockerignore`, so this also
/// returns empty patterns.
pub(super) fn local_build_context_patterns(
	base_dir: &Path,
	service: &crate::compose::types::Service,
) -> (Option<PathBuf>, Vec<String>) {
	let build = match &service.build {
		Some(b) => b,
		None => return (None, Vec::new()),
	};
	let ctx_str = build.context();
	if is_remote_context(ctx_str) {
		return (None, Vec::new());
	}
	let abs =
		std::fs::canonicalize(base_dir.join(ctx_str)).unwrap_or_else(|_| base_dir.join(ctx_str));
	let (_name, patterns) = read_patterns(&abs);
	(Some(abs), patterns)
}

/// Combine the build-context ignore patterns (matched against the
/// build-context-relative path) with the rule's own patterns (matched against
/// the rule-relative path) into a single last-match-wins decision.
///
/// Each list is evaluated against its own basis: a build-context pattern
/// matches only when the event path sits under the build context (otherwise
/// the strip fails and `ctx_rel` is `None`); the rule's patterns always
/// match against `rule_rel`. The combined last match across both lists is the
/// final decision. `None` means no pattern in either list matched.
pub(in crate::engine) fn evaluate_ignore(
	rule_patterns: &[String],
	ctx_patterns: &[String],
	ctx_rel: Option<&str>,
	rule_rel: &str,
) -> Option<bool> {
	let mut last: Option<bool> = None;
	if !ctx_patterns.is_empty() {
		if let Some(rel) = ctx_rel {
			if let Some(d) = evaluate(rel, ctx_patterns) {
				last = Some(d);
			}
		}
	}
	if let Some(d) = evaluate(rule_rel, rule_patterns) {
		last = Some(d);
	}
	last
}

/// Per-rule context shared between the ignore and include pipelines.
///
/// Carries the rule and (optional) build-context absolute paths so the
/// per-event evaluation can strip the changed path against them, plus the
/// identity and dedup of the legacy warning.
pub(in crate::engine) struct RuleContext<'a> {
	pub rule_abs: &'a Path,
	pub ctx_abs: Option<&'a Path>,
	pub base_dir: &'a Path,
	pub service_name: &'a str,
	pub rule_path: &'a str,
	pub ctx_patterns: &'a [String],
	pub warned: &'a mut HashSet<(String, String)>,
}

/// Per-rule decision for one watch event: spec-driven ignore, then legacy
/// project-relative fallback when the spec matcher had nothing to say. A
/// `Some(false)` decision is authoritative and the fallback never overrides
/// it; the fallback only runs when the spec matcher returned `None`.
pub(in crate::engine) fn ignored_with_fallback(
	path: &Path,
	ignore: &[String],
	ctx: &mut RuleContext<'_>,
) -> bool {
	let rule_rel = path
		.strip_prefix(ctx.rule_abs)
		.unwrap_or(path)
		.to_string_lossy()
		.replace('\\', "/");
	let ctx_rel = ctx
		.ctx_abs
		.and_then(|root| path.strip_prefix(root).ok())
		.map(|p| p.to_string_lossy().replace('\\', "/"));
	match evaluate_ignore(ignore, ctx.ctx_patterns, ctx_rel.as_deref(), &rule_rel) {
		Some(d) => d,
		None => legacy_ignore_fallback(
			path,
			ctx.base_dir,
			ctx.service_name,
			ctx.rule_path,
			ignore,
			ctx.warned,
		),
	}
}

/// Per-rule decision for one watch event's include list: spec-driven
/// include, then legacy project-relative fallback. The empty include list
/// means "no include filter" and returns `true` without consulting either
/// matcher.
pub(in crate::engine) fn included_with_fallback(
	path: &Path,
	include: &[String],
	ctx: &mut RuleContext<'_>,
) -> bool {
	if include.is_empty() {
		return true;
	}
	let rule_rel = path
		.strip_prefix(ctx.rule_abs)
		.unwrap_or(path)
		.to_string_lossy()
		.replace('\\', "/");
	match evaluate(&rule_rel, include) {
		Some(d) => d,
		None => legacy_include_fallback(
			path,
			ctx.base_dir,
			ctx.service_name,
			ctx.rule_path,
			include,
			ctx.warned,
		),
	}
}

/// Suggest a rewritten pattern when the legacy fallback matched only because
/// the user wrote the rule's project-relative path into the pattern. If
/// `pattern` starts with the rule's path (after stripping a leading `./`) plus
/// `/`, the suggestion drops that prefix; otherwise the rule's path is
/// mentioned verbatim so the user knows where the pattern is now read against.
pub(in crate::engine) fn legacy_pattern_suggestion(pattern: &str, rule_path: &str) -> String {
	let prefix = rule_path.trim_start_matches("./");
	let with_sep = format!("{prefix}/");
	if pattern.starts_with(&with_sep) {
		let rewritten = &pattern[with_sep.len()..];
		if rewritten.is_empty() {
			format!("the pattern reads against the rule path \"{rule_path}\"")
		} else {
			format!("write \"{rewritten}\" instead")
		}
	} else {
		format!("the pattern is read relative to the rule path \"{rule_path}\"")
	}
}

/// Run the legacy project-relative `ignore` matcher on `path`, warning once
/// per (service, pattern) that decides the path should be ignored.
///
/// Returns `true` if any pattern matches the project-relative path, `false`
/// otherwise. A pattern that has already produced a warning in this watch
/// session still decides; only the warning is deduplicated, so the
/// per-event dispatch sees the correct decision.
pub(in crate::engine) fn legacy_ignore_fallback(
	path: &Path,
	base_dir: &Path,
	service_name: &str,
	rule_path: &str,
	patterns: &[String],
	warned: &mut HashSet<(String, String)>,
) -> bool {
	let proj_rel = path
		.strip_prefix(base_dir)
		.unwrap_or(path)
		.to_string_lossy();
	let mut ignored = false;
	for pat in patterns {
		if legacy_project_relative_ignored(&proj_rel, std::slice::from_ref(pat)) {
			ignored = true;
			if warned.insert((service_name.to_string(), pat.clone())) {
				warn!(
					"watch: service \"{service_name}\": ignore pattern \"{pat}\" only matched relative to the project; compose reads it relative to the rule's path \"{rule_path}\", {}",
					legacy_pattern_suggestion(pat, rule_path)
				);
			}
		}
	}
	ignored
}

/// Run the legacy project-relative `include` matcher on `path`, warning once
/// per (service, pattern) that matched. Returns `true` if any pattern matched.
pub(in crate::engine) fn legacy_include_fallback(
	path: &Path,
	base_dir: &Path,
	service_name: &str,
	rule_path: &str,
	patterns: &[String],
	warned: &mut HashSet<(String, String)>,
) -> bool {
	let proj_rel = path
		.strip_prefix(base_dir)
		.unwrap_or(path)
		.to_string_lossy();
	let mut matched = false;
	for pat in patterns {
		if legacy_project_relative_included(&proj_rel, std::slice::from_ref(pat)) {
			matched = true;
			if warned.insert((service_name.to_string(), pat.clone())) {
				warn!(
					"watch: service \"{service_name}\": include pattern \"{pat}\" only matched relative to the project; compose reads it relative to the rule's path \"{rule_path}\", {}",
					legacy_pattern_suggestion(pat, rule_path)
				);
			}
		}
	}
	matched
}
