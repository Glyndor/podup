//! A host-side removal has to be mirrored in the container, and a removed
//! directory has to be removed with its subtree.
//!
//! `rm -f` refuses a directory ("Is a directory", exit 1), so a removed
//! subdirectory stayed inside the container and the action failed. The
//! removal of the rule's own target directory is still refused: that is the
//! entire mapped area, not an entry delete.

use std::collections::HashSet;
use std::path::Path;

use crate::engine::fake_podman;
use crate::engine::Engine;

use super::super::filtering::events::SyncOp;
use super::super::placement::{join_container_path, plan_remove_placement, RuleShape};

/// A fake that answers the exec round trip the removal runs through, and
/// records the argv it was asked to run.
fn fake_podman() -> fake_podman::FakePodman {
	fake_podman::start(|method, target| {
		if method == "POST" && target.ends_with("/exec") {
			(200, r#"{"Id":"exec-1"}"#.to_string())
		} else if method == "POST" && target.ends_with("/start") {
			(200, String::new())
		} else if method == "GET" && target.contains("/exec/") {
			(200, r#"{"ExitCode":0}"#.to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	})
}

/// The argv of every exec the fake was asked to run.
fn exec_argvs(fake: &fake_podman::FakePodman) -> Vec<Vec<String>> {
	fake.bodies
		.lock()
		.unwrap()
		.iter()
		.filter_map(|body| {
			let value: serde_json::Value = serde_json::from_slice(body).ok()?;
			value
				.get("Cmd")
				.and_then(|cmd| serde_json::from_value(cmd.clone()).ok())
		})
		.collect()
}

/// The removal of a directory under a directory rule has to reach the
/// container as a recursive delete, or the directory stays there.
#[tokio::test]
async fn removing_a_directory_entry_issues_a_recursive_delete() {
	let dir = tempfile::tempdir().unwrap();
	let src = dir.path().join("src");
	std::fs::create_dir(&src).unwrap();
	// The entry is already gone from the host: that is what a removal
	// event means.
	let removed = src.join("modulo");

	let fake = fake_podman();
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), dir.path().to_path_buf());
	let mut ensured = HashSet::new();
	engine
		.dispatch_sync(
			"proj-web-1",
			&src,
			&removed,
			"/app/src",
			&mut ensured,
			&[],
			&src,
			RuleShape::Directory,
			Ok(SyncOp::Remove),
		)
		.await
		.unwrap();

	let argvs = exec_argvs(&fake);
	assert_eq!(
		argvs.len(),
		1,
		"the removal has to run one command: {argvs:?}"
	);
	assert_eq!(
		argvs[0],
		vec!["rm", "-rf", "--", "/app/src/modulo"],
		"a removed directory has to be deleted with its subtree"
	);
}

/// The removal of the rule's own target directory is refused: it is the
/// whole mapped area, not an entry inside it.
#[tokio::test]
async fn removing_the_rule_target_directory_is_refused() {
	let dir = tempfile::tempdir().unwrap();
	let src = dir.path().join("src");
	std::fs::create_dir(&src).unwrap();

	let fake = fake_podman();
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), dir.path().to_path_buf());
	let mut ensured = HashSet::new();
	let error = engine
		.dispatch_sync(
			"proj-web-1",
			&src,
			&src,
			"/app/src",
			&mut ensured,
			&[],
			&src,
			RuleShape::Directory,
			Ok(SyncOp::Remove),
		)
		.await
		.unwrap_err()
		.to_string();
	assert!(
		error.contains("refusing to remove the rule target directory"),
		"{error}"
	);
	assert!(
		exec_argvs(&fake).is_empty(),
		"nothing may be deleted in the container: {:?}",
		exec_argvs(&fake)
	);
}

/// The rule's shape is read when the session starts. A rule whose whole
/// root was deleted no longer looks like a directory on the host, and
/// reading the shape from the filesystem then let the removal of the rule's
/// own target through.
#[test]
fn a_removal_of_the_rule_root_is_refused_after_the_root_is_gone() {
	let root = Path::new("/proj/src");
	// The root is gone: `rule_shape` would now answer "single file", and
	// the placement would name the target itself.
	let placement = plan_remove_placement(RuleShape::Directory, root, root, "/app/src");
	assert_eq!(placement.dest_dir, "/app/src");
	assert_eq!(placement.entry_name, "");
	assert_eq!(
		join_container_path(&placement),
		placement.dest_dir,
		"the removal of the rule's own target directory resolves to the target"
	);
}
