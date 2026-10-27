//! Rule selection for a changed path.
//!
//! Two rules can watch the same path (two services syncing the same
//! directory into their own targets, or one service declaring two rules
//! over it), and every one of them has to run: the first match used to end
//! the scan, so the second service never saw the change.
//!
//! The whole-container actions (`rebuild`, `restart`) keep collapsing per
//! batch, because their effect is the container rather than the file.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::compose::types::{WatchAction, WatchRule};
#[cfg(unix)]
use crate::engine::fake_podman;
use crate::engine::Engine;

use super::super::{RuleEntry, RuleShape, WatchState};

/// A rule entry as the watch loop builds it: a service, its container, and
/// the rule that says what to do with a change.
fn entry(
	service: &str,
	container: &str,
	root: &Path,
	action: WatchAction,
	target: Option<&str>,
) -> RuleEntry {
	RuleEntry {
		service_name: service.to_string(),
		container_name: container.to_string(),
		rule: WatchRule {
			path: "./src".to_string(),
			action,
			target: target.map(str::to_string),
			..Default::default()
		},
		abs_path: root.to_path_buf(),
		build_context_abs: None,
		build_context_patterns: Vec::new(),
		mounts: Arc::new(Vec::new()),
		shape: RuleShape::Directory,
	}
}

fn empty_state() -> WatchState {
	WatchState {
		ensured: HashSet::new(),
		legacy_ignored_warned: HashSet::new(),
		legacy_included_warned: HashSet::new(),
	}
}

#[test]
fn every_rule_watching_the_path_is_selected() {
	let root = Path::new("/proj/src");
	let rules = vec![
		entry(
			"web",
			"proj-web-1",
			root,
			WatchAction::Sync,
			Some("/app/web"),
		),
		entry(
			"api",
			"proj-api-1",
			root,
			WatchAction::Sync,
			Some("/app/api"),
		),
	];
	let mut done = HashSet::new();
	let selected = super::matching_rules(
		&PathBuf::from("/proj/src/main.rs"),
		&rules,
		&mut done,
		&mut |_: &RuleEntry| true,
	);
	assert_eq!(
		selected.len(),
		2,
		"both rules watch the path, so both have to run: {:?}",
		selected.iter().map(|r| &r.service_name).collect::<Vec<_>>()
	);
	assert_eq!(selected[0].service_name, "web");
	assert_eq!(selected[1].service_name, "api");
}

#[test]
fn a_rule_whose_root_does_not_hold_the_path_is_not_selected() {
	let rules = vec![
		entry(
			"web",
			"proj-web-1",
			Path::new("/proj/src"),
			WatchAction::Sync,
			Some("/app/web"),
		),
		entry(
			"api",
			"proj-api-1",
			Path::new("/proj/other"),
			WatchAction::Sync,
			Some("/app/api"),
		),
	];
	let mut done = HashSet::new();
	let selected = super::matching_rules(
		&PathBuf::from("/proj/src/main.rs"),
		&rules,
		&mut done,
		&mut |_: &RuleEntry| true,
	);
	assert_eq!(
		selected.len(),
		1,
		"only the rule whose root holds the path matches"
	);
	assert_eq!(selected[0].service_name, "web");
}

#[test]
fn a_rule_the_filters_reject_is_not_selected() {
	let rules = vec![
		entry(
			"web",
			"proj-web-1",
			Path::new("/proj/src"),
			WatchAction::Sync,
			Some("/app/web"),
		),
		entry(
			"api",
			"proj-api-1",
			Path::new("/proj/src"),
			WatchAction::Sync,
			Some("/app/api"),
		),
	];
	let mut done = HashSet::new();
	let selected = super::matching_rules(
		&PathBuf::from("/proj/src/main.rs"),
		&rules,
		&mut done,
		&mut |entry: &RuleEntry| entry.service_name == "web",
	);
	assert_eq!(selected.len(), 1, "a rule the filters reject does not run");
	assert_eq!(selected[0].service_name, "web");
}

#[test]
fn a_repeated_rebuild_collapses_while_the_other_rules_still_run() {
	let rules = vec![
		entry(
			"web",
			"proj-web-1",
			Path::new("/proj/src"),
			WatchAction::Rebuild,
			None,
		),
		entry(
			"web",
			"proj-web-1",
			Path::new("/proj/src"),
			WatchAction::Rebuild,
			None,
		),
		entry(
			"api",
			"proj-api-1",
			Path::new("/proj/src"),
			WatchAction::Sync,
			Some("/app/api"),
		),
	];
	let mut done = HashSet::new();
	let selected = super::matching_rules(
		&PathBuf::from("/proj/src/main.rs"),
		&rules,
		&mut done,
		&mut |_: &RuleEntry| true,
	);
	assert_eq!(
		selected.len(),
		2,
		"the second rebuild of the same service collapses, the sync rule does not: {:?}",
		selected
			.iter()
			.map(|r| r.rule.action.as_token())
			.collect::<Vec<_>>()
	);

	// The next path of the same batch: the rebuild already ran, and the
	// sync rule still has to run for this path.
	let selected = super::matching_rules(
		&PathBuf::from("/proj/src/other.rs"),
		&rules,
		&mut done,
		&mut |_: &RuleEntry| true,
	);
	assert_eq!(
		selected.len(),
		1,
		"the rebuild already ran in this batch, the sync rule still runs: {:?}",
		selected
			.iter()
			.map(|r| r.rule.action.as_token())
			.collect::<Vec<_>>()
	);
	assert_eq!(selected[0].service_name, "api");
}

/// Two services watching the same directory each get their own copy of the
/// changed file. The dispatch stopped at the first matching rule, so the
/// second service's container never saw the change.
#[tokio::test]
#[cfg(unix)]
async fn two_services_watching_the_same_path_both_sync() {
	let dir = tempfile::tempdir().unwrap();
	let src = dir.path().join("src");
	std::fs::create_dir(&src).unwrap();
	std::fs::write(src.join("main.rs"), b"hello").unwrap();

	let fake = fake_podman::start(|method, _target| {
		if method == "PUT" {
			(200, String::new())
		} else {
			// The per-target `mkdir -p` exec is best-effort; the archive
			// PUT is the part under test.
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), dir.path().to_path_buf());
	let file = crate::parse_str(
		"services:\n  web:\n    image: alpine:latest\n  api:\n    image: alpine:latest\n",
	)
	.unwrap();
	let rules = vec![
		entry(
			"web",
			"proj-web-1",
			&src,
			WatchAction::Sync,
			Some("/app/web"),
		),
		entry(
			"api",
			"proj-api-1",
			&src,
			WatchAction::Sync,
			Some("/app/api"),
		),
	];

	let mut state = empty_state();
	let mut done = HashSet::new();
	engine
		.dispatch_batch(&file, &[src.join("main.rs")], &rules, &mut state, &mut done)
		.await;

	let puts: Vec<String> = fake
		.requests
		.lock()
		.unwrap()
		.iter()
		.filter(|r| r.starts_with("PUT "))
		.cloned()
		.collect();
	assert_eq!(
		puts.len(),
		2,
		"every service watching the path must receive the change: {puts:?}"
	);
	assert!(
		puts.iter()
			.any(|p| p.contains("proj-web-1") && p.contains("%2Fapp%2Fweb")),
		"the first service did not get its copy: {puts:?}"
	);
	assert!(
		puts.iter()
			.any(|p| p.contains("proj-api-1") && p.contains("%2Fapp%2Fapi")),
		"the second service did not get its copy: {puts:?}"
	);
}
