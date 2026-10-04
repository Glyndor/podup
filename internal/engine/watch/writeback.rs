//! Per-path write-back detection for watch rules (#1985).
//!
//! `writes_back` decides whether copying a single changed path into a
//! container would land in a bind mount the rule also watches. The
//! rule-wide check that lived in `bind_mount_feedback` was only able to
//! recognise a few shapes; this module knows the bind sources per target
//! and answers the question for any container path.
//!
//! ## Inputs
//!
//! - `EffectiveMount`: one entry per service volume/target, recording the
//!   in-container path and, for binds, the canonicalised host source and
//!   the read-only flag. Named volumes and tmpfs entries carry
//!   `bind_source: None` and are inert here (a copy into them does not
//!   touch the host).
//! - `container_path`: the in-container path the sync would write or
//!   remove, computed from the host path and the rule target through
//!   `plan_sync_placement` + `join_container_path`.
//! - `watched_root`: the rule's absolute host path, canonicalised by the
//!   caller (the watch loop canonicalises on entry).
//!
//! ## Algorithm
//!
//! Pick the mount whose `target` is the **longest** component-wise prefix
//! of `container_path`. If that mount is not a bind, or is read-only, the
//! answer is `None`. Otherwise the host path the copy would land on is
//! `bind_source.join(container_path minus target)`, and the function
//! returns `Some(host_path)` when that host path equals `watched_root` or
//! sits under it (component-wise).
//!
//! The "longest prefix" step is what closes the carve-out cases: a more
//! specific named volume or tmpfs mounted under a wider bind takes
//! precedence for paths that land inside the carve-out, so the bind is
//! not the write-back target there. The `bind_feedback` check that lived
//! in `placement` only recognised one shape of this; the per-path check
//! handles every component.
//!
//! ## Side-effect-free
//!
//! No filesystem or daemon calls. `effective_mounts` does canonicalise
//! the bind source once at watch start so the per-path comparison is
//! lexical; a canonicalisation failure falls back to the resolved path.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tracing::debug;

use crate::compose::types::Service;
use crate::engine::container::resolve_bind_source;
use crate::engine::volume_mounts::{bind_mounts, mount_targets};
use crate::error::{ComposeError, Result};

use super::events;
use super::placement::{
	container_rel, join_container_path, normalise_container_path, plan_sync_placement,
	SyncPlacement,
};
use super::Engine;

/// One effective mount the watch loop considers when answering
/// `writes_back`. The vector is computed once per service at watch start
/// and shared by every rule of that service.
#[derive(Debug, Clone)]
pub(super) struct EffectiveMount {
	/// Container-side path of the mount. Compared component-wise against
	/// the in-container path of the change.
	pub(super) target: String,
	/// Canonicalised host path for bind mounts, `None` for named volumes
	/// and tmpfs entries (a copy into them never touches the host).
	pub(super) bind_source: Option<PathBuf>,
	/// Parsed read-only flag from the bind. `false` for non-bind entries.
	pub(super) read_only: bool,
}

/// Build one `EffectiveMount` per `mount_targets(service)` entry, filling
/// `bind_source` and `read_only` from the matching `BindMountRef` when the
/// target is a bind. The walk goes through `mount_targets` so a tmpfs
/// without a corresponding bind entry shows up as `bind_source: None`,
/// the same way a named volume does; both are skipped by `writes_back`.
pub(super) fn effective_mounts(service: &Service, base_dir: &Path) -> Vec<EffectiveMount> {
	let binds = bind_mounts(service);
	let mut out: Vec<EffectiveMount> = Vec::with_capacity(mount_targets(service).len());
	for target in mount_targets(service) {
		if let Some(b) = binds.iter().find(|b| b.target == target) {
			let resolved = resolve_bind_source(&b.source, base_dir);
			let canon =
				std::fs::canonicalize(&resolved).unwrap_or_else(|_| PathBuf::from(&resolved));
			out.push(EffectiveMount {
				target,
				bind_source: Some(canon),
				read_only: b.read_only,
			});
		} else {
			out.push(EffectiveMount {
				target,
				bind_source: None,
				read_only: false,
			});
		}
	}
	out
}

/// True when a copy of `container_path` into the container would land on
/// the host at a path under `watched_root`, i.e. when the watcher would
/// pick the change back up and loop. Returns the host path the copy
/// would land on when it does write back, `None` otherwise.
///
/// Component-wise prefixes throughout: `/app` covers `/app/src` but not
/// `/application`. A path inside a more specific mount (a volume or a
/// tmpfs under the bind) does not write back, even when the wider bind
/// does; the longest matching `target` wins. A read-only bind cannot
/// write back, so it also returns `None`.
pub(super) fn writes_back(
	mounts: &[EffectiveMount],
	container_path: &str,
	watched_root: &Path,
) -> Option<PathBuf> {
	let normalised = normalise_container_path("/", container_path);
	let container_parts = path_components(&normalised);
	// Longest component-wise prefix wins; a mount at `/` has length 0 and
	// covers every path.
	let mut best: Option<(usize, &EffectiveMount)> = None;
	for m in mounts {
		let mount_target = normalise_container_path("/", &m.target);
		let mount_parts = path_components(&mount_target);
		if mount_parts.len() > container_parts.len() {
			continue;
		}
		if container_parts[..mount_parts.len()] != mount_parts[..] {
			continue;
		}
		if best.is_none_or(|(len, _)| mount_parts.len() > len) {
			best = Some((mount_parts.len(), m));
		}
	}
	let (best_len, mount) = best?;
	let bind_source = mount.bind_source.as_ref()?;
	if mount.read_only {
		return None;
	}
	let stripped: Vec<&str> = container_parts[best_len..].to_vec();
	let mut host_path = bind_source.clone();
	for component in &stripped {
		host_path.push(component);
	}
	if host_under_watched(&host_path, watched_root) {
		Some(host_path)
	} else {
		None
	}
}

/// True when `host_path` equals `watched_root` or sits under it,
/// component-wise (lexical, the canonicalised form is the caller's
/// responsibility: the loop canonicalises `watched_root` on entry and
/// `effective_mounts` canonicalises bind sources).
fn host_under_watched(host_path: &Path, watched_root: &Path) -> bool {
	let host_parts = path_components_host(host_path);
	let root_parts = path_components_host(watched_root);
	if host_parts.len() < root_parts.len() {
		return false;
	}
	host_parts[..root_parts.len()] == root_parts[..]
}

/// Split a container-style absolute path into its non-empty components.
/// `/` produces an empty vec, which is how the "longest prefix" walk
/// recognises a root mount that covers every container path.
fn path_components(path: &str) -> Vec<&str> {
	path_components_iter(path.split('/'))
}

/// Same shape for a host `Path`. Components are the `OsStr` rendered as
/// `&str`; non-UTF-8 paths would silently not match, but every host path
/// the rule ever sees comes from `effective_mounts` (canonicalised from
/// the compose file) or from the watcher's own canonicalisation, both of
/// which are UTF-8.
fn path_components_host(path: &Path) -> Vec<String> {
	let mut out = Vec::new();
	for c in path.components() {
		// `Component::as_os_str` is `&OsStr`; the only way it lands in
		// here is a UTF-8 path (compose file paths are UTF-8 on Unix and
		// Windows alike; canonicalisation does not change that), so the
		// `unwrap` is the right shape.
		let s = c.as_os_str().to_str().unwrap_or("").to_string();
		if !s.is_empty() {
			out.push(s);
		}
	}
	out
}

fn path_components_iter<'a, I: IntoIterator<Item = &'a str>>(parts: I) -> Vec<&'a str> {
	parts.into_iter().filter(|c| !c.is_empty()).collect()
}

impl Engine {
	/// Run the sync step for a rule unless the change would write back into
	/// the watched tree through one of the service's bind mounts. The
	/// restart, rebuild or exec part of a `sync+restart` / `sync+exec`
	/// action still runs; this only gates the sync upload/remove.
	pub(in crate::engine::watch) async fn maybe_sync(
		&self,
		entry: &super::RuleEntry,
		path: &Path,
		ensured: &mut HashSet<(String, String)>,
	) -> Result<()> {
		let Some(target) = &entry.rule.target else {
			return Ok(());
		};
		// Uploads filter every archive entry, but a removal runs `rm` on one
		// container path; when that path maps back into the watched tree it
		// is another host file, so leave it alone.
		if matches!(events::sync_op_for(path), Ok(events::SyncOp::Remove)) {
			let placement = plan_sync_placement(&entry.abs_path, path, target);
			let container_path = join_container_path(&SyncPlacement {
				entry_name: container_rel(Path::new(&placement.entry_name)),
				dest_dir: placement.dest_dir,
			});
			if let Some(host) = writes_back(&entry.mounts, &container_path, &entry.abs_path) {
				debug!(
					"skip removal of {container_path}: it is bind-mounted back to {}",
					host.display()
				);
				return Ok(());
			}
		}
		self.dispatch_sync(
			&entry.container_name,
			&entry.abs_path,
			path,
			target,
			ensured,
			&entry.mounts,
			&entry.abs_path,
		)
		.await
	}

	/// Pick the right sync dispatch for the changed path: copy on a change,
	/// remove on a deletion. The decision is taken from the host filesystem
	/// rather than the event kind so a debounce batch that contains a write
	/// and a remove of the same file inside 100 ms dispatches each path
	/// correctly: the file the host still has gets uploaded, the file the
	/// host has dropped gets removed. Sync-family actions all funnel
	/// through here so the removal path is owned in one place.
	///
	/// `mounts` and `watched_root` are passed down to the per-entry filter
	/// that drops descendants whose container path lands in a deeper bind
	/// that maps back into the watched tree. The earlier rule-wide check
	/// (`writes_back` on the changed path alone) is a special case of the
	/// per-entry check: the root is also evaluated, and a directory that
	/// matches the loop pattern only on a non-root descendant still
	/// produces a sync that drops the loop-causing entry while keeping
	/// every safe entry.
	#[allow(clippy::too_many_arguments)]
	pub(in crate::engine::watch) async fn dispatch_sync(
		&self,
		container: &str,
		root: &Path,
		changed: &Path,
		target: &str,
		ensured: &mut HashSet<(String, String)>,
		mounts: &[EffectiveMount],
		watched_root: &Path,
	) -> Result<()> {
		match events::sync_op_for(changed) {
			Ok(events::SyncOp::Upload) => {
				self.sync_to_container(
					container,
					root,
					changed,
					target,
					ensured,
					mounts,
					watched_root,
				)
				.await
			}
			Ok(events::SyncOp::Remove) => {
				self.remove_from_container(container, root, changed, target)
					.await
			}
			Err(e) => Err(ComposeError::Watch(format!(
				"cannot read {}: {e}; leaving the container copy as it is",
				changed.display()
			))),
		}
	}
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "writeback_tests.rs"]
mod tests;
