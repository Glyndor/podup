//! Pure host-path → container placement and watch-event helpers.
//!
//! These functions hold the side-effect-free decisions of the watch engine:
//! mapping a changed host path to its container archive placement, filtering
//! which notify events drive a sync, validating that sync rules carry a target,
//! and bookkeeping for the per-target `mkdir`. Keeping them here lets the
//! dispatch loop in [`super`] stay focused on I/O.

use std::collections::HashSet;
use std::path::Path;

use crate::compose::types::WatchRule;
use crate::error::{ComposeError, Result};

/// Where a changed host path lands inside the container for a `sync` action:
/// the archive entry name and the directory the tar is extracted at.
pub(super) struct SyncPlacement {
	/// Archive path the changed entry occupies inside the tar.
	pub(super) entry_name: String,
	/// Container directory the archive is PUT (extracted) at.
	pub(super) dest_dir: String,
}

/// Map a changed host path to its container archive placement, matching
/// docker-compose `watch` semantics.
///
/// `root` is the watch rule's absolute host path, `changed` the path that
/// actually changed (equal to `root` for a single-file rule, a descendant for a
/// directory rule), and `target` the rule's container target.
///
/// For a directory rule the changed entry keeps its path relative to `root`
/// (subdirectories preserved) and is extracted under `target` treated as a
/// directory. For a single-file rule the entry is stored under
/// `basename(target)` and extracted into `target`'s parent, so a renaming
/// target is honoured.
pub(super) fn plan_sync_placement(root: &Path, changed: &Path, target: &str) -> SyncPlacement {
	if root.is_dir() {
		// Directory rule: preserve the changed file's subpath under `target`,
		// which is treated as a directory.
		let rel = changed.strip_prefix(root).unwrap_or(changed);
		let entry_name = rel.to_string_lossy().into_owned();
		let dest_dir = target.trim_end_matches('/').to_string();
		let dest_dir = if dest_dir.is_empty() {
			"/".to_string()
		} else {
			dest_dir
		};
		SyncPlacement {
			entry_name,
			dest_dir,
		}
	} else {
		// Single-file rule: store under the target basename so a renaming target
		// is honoured, and extract into the target's parent directory.
		let target_path = Path::new(target);
		let entry_name = target_path
			.file_name()
			.map(|n| n.to_string_lossy().into_owned())
			.or_else(|| {
				changed
					.file_name()
					.map(|n| n.to_string_lossy().into_owned())
			})
			.unwrap_or_default();
		let dest_dir = target_path
			.parent()
			.map(|p| p.to_string_lossy().into_owned())
			.filter(|s| !s.is_empty())
			.unwrap_or_else(|| "/".to_string());
		SyncPlacement {
			entry_name,
			dest_dir,
		}
	}
}

/// True when a notify event should drive a watch action.
///
/// docker-compose `watch` only reacts to write/create/remove/rename changes. The
/// vendored notify inotify backend also emits `Access` events (it sets
/// `WatchMask::OPEN`), so merely opening/reading a watched file would otherwise
/// fire a sync — and the sync's own read of the source re-opens the path,
/// generating fresh `Access` events that feed back into another sync. Filtering
/// to create/modify/remove (rename is a `Modify(Name(..))`) matches compose
/// semantics and breaks that feedback loop.
pub(super) fn is_dispatch_event(kind: &notify::EventKind) -> bool {
	use notify::EventKind;
	matches!(
		kind,
		EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
	)
}

/// Reject a watch rule whose action needs a `target` but has none. docker
/// compose treats a sync rule without a target as a configuration error rather
/// than silently performing no sync.
pub(super) fn validate_sync_target(rule: &WatchRule) -> Result<()> {
	if rule.action.requires_target() && rule.target.is_none() {
		return Err(ComposeError::Watch(format!(
			"watch rule for '{}' uses a sync action ({}) but has no target",
			rule.path,
			rule.action.as_token()
		)));
	}
	Ok(())
}

/// Best-effort `mkdir -p` argv for creating a sync target directory. The `--`
/// terminates options so a target beginning with `-` (e.g. `-m0777`) is treated
/// as a path, not parsed as a flag by busybox `mkdir`.
pub(super) fn mkdir_p_argv(dest_dir: &str) -> Vec<String> {
	vec![
		"mkdir".into(),
		"-p".into(),
		"--".into(),
		dest_dir.to_string(),
	]
}

/// Record that `(container, dest)` has had its directory ensured, returning
/// `true` the first time (the caller should then issue the `mkdir`) and `false`
/// thereafter so the per-event `mkdir` exec is issued at most once per target.
pub(super) fn mark_dir_ensured(
	ensured: &mut HashSet<(String, String)>,
	container: &str,
	dest: &str,
) -> bool {
	ensured.insert((container.to_string(), dest.to_string()))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::{
		is_dispatch_event, mark_dir_ensured, mkdir_p_argv, plan_sync_placement,
		validate_sync_target,
	};
	use crate::compose::types::{WatchAction, WatchRule};
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
}
