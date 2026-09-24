//! Pure host-path → container placement and watch-event helpers.
//!
//! These functions hold the side-effect-free decisions of the watch engine:
//! mapping a changed host path to its container archive placement, filtering
//! which notify events drive a sync, validating that sync rules carry a target,
//! and bookkeeping for the per-target `mkdir`. Keeping them here lets the
//! dispatch loop in [`super`] stay focused on I/O.

use std::collections::HashSet;
use std::path::Path;

use crate::compose::types::{Service, WatchRule};
use crate::error::{ComposeError, Result};

/// Where a changed host path lands inside the container for a `sync` action:
/// the archive entry name and the directory the tar is extracted at.
pub(super) struct SyncPlacement {
	/// Archive path the changed entry occupies inside the tar.
	pub(super) entry_name: String,
	/// Container directory the archive is PUT (extracted) at.
	pub(super) dest_dir: String,
}

/// Resolve the container-side path a removed host entry should be deleted
/// from, mirroring [`plan_sync_placement`] for a `Remove` event.
///
/// A `sync` rule's `target` is, in the container, where the changed entries
/// live. A removal on the host has to be reflected by a `DELETE` against the
/// matching path inside the container, so the archive `entry_name` and `dir`
/// computed for the corresponding add/modify are the same values needed here.
/// The single-file rule re-uses its rename mapping; the directory rule
/// preserves the relative subpath.
///
/// The caller is responsible for refusing removals that target the rule's
/// container directory itself (rather than an entry inside it): a removal of
/// the target directory is a different operation, never an entry delete, and
/// letting it through would `rm -rf` the container's destination.
pub(super) fn plan_remove_placement(root: &Path, removed: &Path, target: &str) -> SyncPlacement {
	plan_sync_placement(root, removed, target)
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
/// fire a sync, and the sync's own read of the source re-opens the path,
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

/// True when the notify event is a removal. The dispatch loop funnels a `sync`
/// action to either an upload or a delete based on this; a `rebuild`/`restart`
/// rule does not consult it.
pub(super) fn is_remove_event(kind: &notify::EventKind) -> bool {
	matches!(kind, notify::EventKind::Remove(_))
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

/// The full container-side path a `SyncPlacement` resolves to, suitable for
/// `DELETE` on the libpod archive endpoint.
///
/// A single-file sync writes one entry (`entry_name`) into one directory
/// (`dest_dir`); the on-disk artifact is `dest_dir/entry_name`. The directory
/// rule's `dest_dir` may already be the final path when `entry_name` is empty
/// (e.g. a removal of the rule root itself), which is precisely the case the
/// delete guard upstream refuses; the joined form mirrors the same shape so
/// the guard and the path agree.
///
/// Pure so the join is unit-testable without a container.
pub(super) fn join_container_path(placement: &SyncPlacement) -> String {
	join_archive_path(&placement.dest_dir, &placement.entry_name)
}

fn join_archive_path(dir: &str, entry: &str) -> String {
	if entry.is_empty() {
		return dir.to_string();
	}
	if dir == "/" {
		format!("/{entry}")
	} else if dir.ends_with('/') {
		format!("{dir}{entry}")
	} else {
		format!("{dir}/{entry}")
	}
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

/// Whether `target` (an absolute container path) sits under one of `mounts`
/// (container-side mount targets), so a sync there does not hit the root
/// filesystem. Compares whole path components: `/app` covers `/app` and
/// `/app/src`, not `/application`. A trailing `/` on either side is ignored;
/// a mount of `/` covers everything.
pub(super) fn target_is_on_a_mount(target: &str, mounts: &[&str]) -> bool {
	let target_parts = path_components(target);
	for mount in mounts {
		let mount_parts = path_components(mount);
		if mount_parts.is_empty() || target_parts.starts_with(&mount_parts) {
			return true;
		}
	}
	false
}

/// The warning to print when a sync `target` of service `service_name` sits on
/// a `read_only: true` root filesystem that no volume or tmpfs covers, or
/// `None` when the service is writable there. Tmpfs entries are cut at their
/// first `:` (they can carry `:size=...` options).
///
/// `volumes_from:` is treated as a third "covered" case even though the
/// function does not resolve it to concrete mount paths: a sibling service's
/// volumes are what `volumes_from` mounts into this container, and resolving
/// them would require walking the compose graph here, which `read_only` checks
/// do not need to do. Without this short-circuit the warning would routinely
/// fire for a service that actually does have its target covered by a
/// `volumes_from` reference, which is a false positive the user would have to
/// learn to ignore.
pub(super) fn read_only_target_warning(
	service_name: &str,
	service: &Service,
	target: &str,
) -> Option<String> {
	if service.read_only != Some(true) {
		return None;
	}
	if !service.volumes_from.is_empty() {
		return None;
	}
	let mut mounts: Vec<&str> = service.volumes.iter().map(|v| v.target()).collect();
	let tmpfs_paths: Vec<String> = service
		.tmpfs
		.to_list()
		.into_iter()
		.map(|s| s.split(':').next().unwrap_or("").to_string())
		.collect();
	mounts.extend(tmpfs_paths.iter().map(String::as_str));
	if target_is_on_a_mount(target, &mounts) {
		return None;
	}
	Some(format!(
		"{service_name}: sync target {target} is on the read-only root filesystem (read_only: true) and no volume or tmpfs covers it, so every sync to it will fail; mount a volume or tmpfs at {target}"
	))
}

/// Split an absolute container path into its non-empty components, with any
/// trailing slash stripped. `/` and `//` produce an empty vec, which is how
/// `target_is_on_a_mount` recognises a root mount that covers everything.
fn path_components(path: &str) -> Vec<&str> {
	path.trim_end_matches('/')
		.split('/')
		.filter(|c| !c.is_empty())
		.collect()
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "placement_tests.rs"]
mod tests;
