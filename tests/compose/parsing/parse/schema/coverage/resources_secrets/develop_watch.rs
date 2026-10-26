//! `develop.watch` and its three actions.
//!
//! Split out of `tests/compose/parsing/parse/coverage/resources_secrets/mod.rs`; the imports
//! are inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// develop.watch
// ---------------------------------------------------------------------------

#[test]
fn develop_watch_sync() {
	let yaml = r#"
services:
  app:
    image: alpine
    develop:
      watch:
        - path: ./src
          action: sync
          target: /app/src
          ignore:
            - node_modules
"#;
	let svc = &parse_str(yaml).unwrap().services["app"];
	let dev = svc.develop.as_ref().unwrap();
	assert_eq!(dev.watch.len(), 1);
	let rule = &dev.watch[0];
	assert_eq!(rule.path, "./src");
	assert_eq!(rule.action, WatchAction::Sync);
	assert_eq!(rule.target.as_deref(), Some("/app/src"));
	assert_eq!(rule.ignore.len(), 1);
}

#[test]
fn develop_watch_rebuild() {
	let yaml = r#"
services:
  app:
    image: alpine
    develop:
      watch:
        - path: ./Dockerfile
          action: rebuild
"#;
	let svc = &parse_str(yaml).unwrap().services["app"];
	let rule = &svc.develop.as_ref().unwrap().watch[0];
	assert_eq!(rule.action, WatchAction::Rebuild);
}

#[test]
fn develop_watch_sync_restart() {
	let yaml = r#"
services:
  app:
    image: alpine
    develop:
      watch:
        - path: ./config
          action: sync+restart
          target: /etc/app
          initial_sync: true
"#;
	let file = parse_str(yaml).unwrap();
	let rule = &file.services["app"].develop.as_ref().unwrap().watch[0];
	assert_eq!(rule.action, WatchAction::SyncAndRestart);
	assert!(rule.initial_sync);
}
