use super::{
	is_dispatch_event, is_remove_event, join_container_path, mark_dir_ensured, mkdir_p_argv,
	plan_remove_placement, plan_sync_placement, read_only_target_warning, target_is_on_a_mount,
	validate_sync_target,
};
use crate::compose::types::{Service, WatchAction, WatchRule};
use std::collections::HashSet;
use std::fs;
use tempfile::tempdir;

fn rule(action: WatchAction, target: Option<&str>) -> WatchRule {
	WatchRule {
		path: "src".into(),
		action,
		target: target.map(str::to_string),
		..Default::default()
	}
}

#[test]
fn dispatch_event_filters_access_and_other() {
	use notify::event::{AccessKind, CreateKind, ModifyKind, RemoveKind};
	use notify::EventKind;
	assert!(is_dispatch_event(&EventKind::Create(CreateKind::File)));
	assert!(is_dispatch_event(&EventKind::Modify(ModifyKind::Any)));
	assert!(is_dispatch_event(&EventKind::Remove(RemoveKind::File)));
	// Access (read/open) and Other/Any must not trigger a sync.
	assert!(!is_dispatch_event(&EventKind::Access(AccessKind::Open(
		notify::event::AccessMode::Read
	))));
	assert!(!is_dispatch_event(&EventKind::Access(AccessKind::Any)));
	assert!(!is_dispatch_event(&EventKind::Other));
	assert!(!is_dispatch_event(&EventKind::Any));
}

#[test]
fn validate_sync_target_rejects_targetless_sync() {
	assert!(validate_sync_target(&rule(WatchAction::Sync, None)).is_err());
	assert!(validate_sync_target(&rule(WatchAction::SyncAndRestart, None)).is_err());
	assert!(validate_sync_target(&rule(WatchAction::SyncAndExec, None)).is_err());
}

#[test]
fn validate_sync_target_accepts_target_and_whole_container_actions() {
	assert!(validate_sync_target(&rule(WatchAction::Sync, Some("/app"))).is_ok());
	// rebuild/restart need no target.
	assert!(validate_sync_target(&rule(WatchAction::Rebuild, None)).is_ok());
	assert!(validate_sync_target(&rule(WatchAction::Restart, None)).is_ok());
}

#[test]
fn mkdir_argv_terminates_options_for_leading_dash_target() {
	// A target beginning with `-` must be passed as a path, not a flag.
	assert_eq!(mkdir_p_argv("-m0777"), vec!["mkdir", "-p", "--", "-m0777"]);
	assert_eq!(mkdir_p_argv("/app"), vec!["mkdir", "-p", "--", "/app"]);
}

#[test]
fn mark_dir_ensured_only_first_time_per_target() {
	let mut ensured: HashSet<(String, String)> = HashSet::new();
	// First time for a (container, dest) returns true (issue the mkdir)...
	assert!(mark_dir_ensured(&mut ensured, "c1", "/app"));
	// ...and subsequent calls for the same pair return false (skip it).
	assert!(!mark_dir_ensured(&mut ensured, "c1", "/app"));
	// A different container or dest is ensured independently.
	assert!(mark_dir_ensured(&mut ensured, "c2", "/app"));
	assert!(mark_dir_ensured(&mut ensured, "c1", "/other"));
}

#[test]
fn placement_directory_rule_preserves_subpath() {
	// A directory rule: a change to <root>/sub/b.txt must keep the `sub/`
	// subpath under the target directory.
	let dir = tempdir().unwrap();
	fs::create_dir(dir.path().join("sub")).unwrap();
	let changed = dir.path().join("sub/b.txt");
	fs::write(&changed, b"b").unwrap();

	let p = plan_sync_placement(dir.path(), &changed, "/app");
	assert_eq!(p.entry_name, "sub/b.txt");
	assert_eq!(p.dest_dir, "/app");
}

#[test]
fn placement_directory_rule_trailing_slash_target() {
	let dir = tempdir().unwrap();
	let changed = dir.path().join("a.txt");
	fs::write(&changed, b"a").unwrap();

	let p = plan_sync_placement(dir.path(), &changed, "/app/");
	assert_eq!(p.entry_name, "a.txt");
	assert_eq!(p.dest_dir, "/app");
}

#[test]
fn placement_single_file_rule_honours_renaming_target() {
	// A single-file rule whose target renames the file must store the entry
	// under the target basename and extract into the target's parent.
	let dir = tempdir().unwrap();
	let src = dir.path().join("settings.yml");
	fs::write(&src, b"k: v").unwrap();

	let p = plan_sync_placement(&src, &src, "/app/config.yml");
	assert_eq!(p.entry_name, "config.yml");
	assert_eq!(p.dest_dir, "/app");
}

#[test]
fn placement_single_file_rule_same_basename() {
	// The existing same-basename case still lands the file at the target.
	let dir = tempdir().unwrap();
	let src = dir.path().join("app.txt");
	fs::write(&src, b"x").unwrap();

	let p = plan_sync_placement(&src, &src, "/newdir/app.txt");
	assert_eq!(p.entry_name, "app.txt");
	assert_eq!(p.dest_dir, "/newdir");
}

#[test]
fn placement_single_file_rule_target_at_root() {
	let dir = tempdir().unwrap();
	let src = dir.path().join("app.txt");
	fs::write(&src, b"x").unwrap();

	let p = plan_sync_placement(&src, &src, "/app.txt");
	assert_eq!(p.entry_name, "app.txt");
	assert_eq!(p.dest_dir, "/");
}

#[test]
fn remove_event_matches_remove_kind_only() {
	use notify::event::{CreateKind, ModifyKind, RemoveKind};
	use notify::EventKind;
	assert!(is_remove_event(&EventKind::Remove(RemoveKind::File)));
	assert!(is_remove_event(&EventKind::Remove(RemoveKind::Folder)));
	assert!(is_remove_event(&EventKind::Remove(RemoveKind::Any)));
	assert!(!is_remove_event(&EventKind::Create(CreateKind::File)));
	assert!(!is_remove_event(&EventKind::Modify(ModifyKind::Any)));
	assert!(!is_remove_event(&EventKind::Other));
	assert!(!is_remove_event(&EventKind::Any));
}

#[test]
fn remove_placement_directory_rule_preserves_subpath() {
	// A removal under a directory rule must keep the same subpath the
	// corresponding add/modify produced, so the DELETE inside the container
	// targets exactly what an upload would have written.
	let dir = tempdir().unwrap();
	fs::create_dir(dir.path().join("sub")).unwrap();
	let removed = dir.path().join("sub/b.txt");
	fs::write(&removed, b"b").unwrap();

	let p = plan_remove_placement(dir.path(), &removed, "/app");
	assert_eq!(p.entry_name, "sub/b.txt");
	assert_eq!(p.dest_dir, "/app");
}

#[test]
fn remove_placement_single_file_rule_honours_renaming_target() {
	// A single-file rule's target renames the file: the deletion must
	// still target the renamed name, not the source basename, so a removal
	// removes the same on-disk artifact the upload created.
	let dir = tempdir().unwrap();
	let src = dir.path().join("settings.yml");
	fs::write(&src, b"k: v").unwrap();

	let p = plan_remove_placement(&src, &src, "/app/config.yml");
	assert_eq!(p.entry_name, "config.yml");
	assert_eq!(p.dest_dir, "/app");
}

#[test]
fn join_container_path_combines_dir_and_entry() {
	// The full container path the DELETE should hit is `dest_dir/entry_name`.
	// The root dir case must not double the separator (`/app`, not `//app`).
	let placement = super::SyncPlacement {
		entry_name: "config.yml".into(),
		dest_dir: "/app".into(),
	};
	assert_eq!(join_container_path(&placement), "/app/config.yml");
	let root = super::SyncPlacement {
		entry_name: "config.yml".into(),
		dest_dir: "/".into(),
	};
	assert_eq!(join_container_path(&root), "/config.yml");
	// An empty entry yields just the directory (the target-dir guard above
	// is what blocks this; the join itself stays well-defined).
	let empty = super::SyncPlacement {
		entry_name: String::new(),
		dest_dir: "/app".into(),
	};
	assert_eq!(join_container_path(&empty), "/app");
}

#[test]
fn target_is_on_a_mount_matches_whole_components() {
	// A mount covers its own path and any deeper path component, but not a
	// sibling with a shared string prefix (`/app` does not cover `/application`).
	assert!(target_is_on_a_mount("/app/src", &["/app"]));
	assert!(target_is_on_a_mount("/app", &["/app"]));
	assert!(target_is_on_a_mount("/app/", &["/app"]));
	assert!(target_is_on_a_mount("/app/src", &["/app/"]));
	assert!(!target_is_on_a_mount("/application", &["/app"]));
	assert!(!target_is_on_a_mount("/app", &["/data"]));
	assert!(!target_is_on_a_mount("/app", &[]));
	// A root mount (`/`) covers every absolute path, regardless of depth.
	assert!(target_is_on_a_mount("/anything", &["/"]));
}

#[test]
fn read_only_target_warning_cases() {
	// `read_only: true` with no volumes or tmpfs: the target sits on the
	// read-only root filesystem, so the warning must fire and mention both
	// the target and the literal `read_only: true` so the user can match
	// it against the compose file (#1897).
	let svc: Service = serde_yaml::from_str("read_only: true\nimage: x\n").unwrap();
	let msg = read_only_target_warning("web", &svc, "/app");
	let msg = msg.expect("warning expected for read_only: true with no mount");
	assert!(msg.contains("/app"));
	assert!(msg.contains("read_only: true"));

	// A bind mount that covers the target: the warning must be suppressed,
	// because every sync lands inside the writable mount, not on the root fs.
	let svc: Service =
		serde_yaml::from_str("read_only: true\nimage: x\nvolumes:\n  - ./src:/app\n").unwrap();
	assert_eq!(read_only_target_warning("web", &svc, "/app/src"), None);

	// A tmpfs at the target (with the usual `:size=10m` option, cut at the
	// first `:`) also makes the destination writable: the warning is suppressed.
	let svc: Service =
		serde_yaml::from_str("read_only: true\nimage: x\ntmpfs:\n  - /app:size=10m\n").unwrap();
	assert_eq!(read_only_target_warning("web", &svc, "/app"), None);

	// `read_only: false` (explicit): the container is writable regardless
	// of mount coverage, so there is nothing to warn about.
	let svc: Service = serde_yaml::from_str("read_only: false\nimage: x\n").unwrap();
	assert_eq!(read_only_target_warning("web", &svc, "/app"), None);

	// `read_only` omitted: same as `false`, the default, no warning.
	let svc: Service = serde_yaml::from_str("image: x\n").unwrap();
	assert_eq!(read_only_target_warning("web", &svc, "/app"), None);
}
