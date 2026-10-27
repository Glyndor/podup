//! The ignore and include lists of a sync rule hold for every file of a
//! directory upload, not only for the path that raised the event.
//!
//! The event loop filters the changed path. A new directory, or a directory
//! event, uploads its whole subtree, and the packer used to apply only the
//! write-back filter to what it found there, so a file the rule ignores
//! (credentials, build output) was uploaded with its neighbours.

use std::collections::HashSet;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use crate::compose::types::{WatchAction, WatchRule};
use crate::engine::fake_podman;
use crate::engine::Engine;

use super::{RuleEntry, RuleShape, WatchState};

fn entry(root: &Path, ignore: &[&str], include: &[&str]) -> RuleEntry {
	RuleEntry {
		service_name: "web".to_string(),
		container_name: "proj-web-1".to_string(),
		rule: WatchRule {
			path: "./src".to_string(),
			action: WatchAction::Sync,
			target: Some("/app".to_string()),
			ignore: ignore.iter().map(|s| s.to_string()).collect(),
			include: include.iter().map(|s| s.to_string()).collect(),
			..Default::default()
		},
		abs_path: root.to_path_buf(),
		build_context_abs: None,
		build_context_patterns: Vec::new(),
		mounts: Arc::new(Vec::new()),
		shape: RuleShape::Directory,
	}
}

/// Names of the entries in the uploaded archive, in upload order.
fn uploaded_names(fake: &fake_podman::FakePodman) -> Vec<String> {
	let requests = fake.requests.lock().unwrap();
	let bodies = fake.bodies.lock().unwrap();
	let put = requests
		.iter()
		.position(|r| r.starts_with("PUT "))
		.expect("the archive was not uploaded");
	let mut tar_bytes = Vec::new();
	flate2::read::GzDecoder::new(bodies[put].as_slice())
		.read_to_end(&mut tar_bytes)
		.expect("the upload is a gzip stream");
	tar::Archive::new(tar_bytes.as_slice())
		.entries()
		.unwrap()
		.map(|e| e.unwrap().path().unwrap().to_string_lossy().into_owned())
		.collect()
}

/// Sync `changed` (a directory under the rule root) through the dispatch
/// the event loop uses and return what was uploaded.
async fn upload(ignore: &[&str], include: &[&str]) -> Vec<String> {
	let dir = tempfile::tempdir().unwrap();
	let src = dir.path().join("src");
	let sub = src.join("sub");
	std::fs::create_dir_all(&sub).unwrap();
	std::fs::write(sub.join("keep.txt"), b"keep").unwrap();
	std::fs::write(sub.join("private.env"), b"secret").unwrap();
	std::fs::write(sub.join("main.rs"), b"fn main() {}").unwrap();

	let fake = fake_podman::start(|method, _target| {
		if method == "PUT" {
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), dir.path().to_path_buf());
	let file = crate::parse_str("services:\n  web:\n    image: alpine:latest\n").unwrap();
	let rules = vec![entry(&src, ignore, include)];
	let mut state = WatchState {
		ensured: HashSet::new(),
		legacy_ignored_warned: HashSet::new(),
		legacy_included_warned: HashSet::new(),
	};
	let mut done = HashSet::new();
	engine
		.dispatch_batch(&file, &[sub.clone()], &rules, &mut state, &mut done)
		.await;
	uploaded_names(&fake)
}

#[tokio::test]
async fn a_directory_upload_leaves_out_the_files_the_rule_ignores() {
	let names = upload(&["**/private.env"], &[]).await;
	assert!(
		names.iter().any(|n| n.ends_with("keep.txt")),
		"the other files must still go up: {names:?}"
	);
	assert!(
		!names.iter().any(|n| n.ends_with("private.env")),
		"an ignored file went up with its directory: {names:?}"
	);
}

/// `initial_sync` uploads the rule root without a watch event to filter it
/// first, so the lists have to hold inside the packer too.
#[tokio::test]
async fn the_initial_sync_sends_only_the_files_the_include_list_names() {
	let dir = tempfile::tempdir().unwrap();
	let src = dir.path().join("src");
	std::fs::create_dir_all(&src).unwrap();
	std::fs::write(src.join("keep.txt"), b"keep").unwrap();
	std::fs::write(src.join("main.rs"), b"fn main() {}").unwrap();

	let fake = fake_podman::start(|method, _target| {
		if method == "PUT" {
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), dir.path().to_path_buf());
	let mut rule = entry(&src, &[], &["*.rs"]);
	rule.rule.initial_sync = true;
	let mut ensured = HashSet::new();
	let failed = engine.sync_all(&[rule], &mut ensured, "initial sync").await;
	assert!(failed.is_empty(), "{failed:?}");
	let names = uploaded_names(&fake);
	assert!(names.iter().any(|n| n.ends_with("main.rs")), "{names:?}");
	assert!(
		!names.iter().any(|n| n.ends_with("keep.txt")),
		"a file the include list does not name went up: {names:?}"
	);
}
