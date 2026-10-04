//! File-watch engine for `develop: watch:` rules.
//!
//! [`Engine::watch`] sets up an `inotify`/`kqueue` watcher via `notify`, then
//! dispatches each change event to the matching [`WatchRule`]. Debouncing
//! collapses rapid bursts into a single action. Actions:
//! - `sync`: tar the changed file and upload it into the container; for a
//!   deletion on the host, mirror the removal into the container
//! - `rebuild`: stop container, rebuild image, restart
//! - `restart`: stop and start the container without rebuilding
//! - `sync+restart`: sync first, then restart
//! - `sync+exec`: sync, then run the rule's `exec` command inside the container
//!
//! The per-rule I/O lives in [`actions`]; placement decisions live in
//! [`placement`]; the write-back loop lives in [`writeback`]. The dispatch
//! loop below only owns the watcher plumbing and the per-path decision.

mod actions;
mod events;
mod ignore_filter;
mod placement;
pub(in crate::engine) mod sync;
#[cfg(feature = "test-helpers")]
mod test_helpers;
mod writeback;

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tokio::sync::Notify;
use tracing::{debug, info, warn};

use crate::compose::types::{ComposeFile, WatchAction, WatchRule};
use crate::error::{ComposeError, Result};

use placement::{
	is_dispatch_event, join_container_path, plan_sync_placement, read_only_target_warning,
	validate_sync_target,
};

use writeback::{effective_mounts, writes_back, EffectiveMount};

use super::Engine;

/// Bound on the in-flight watch-event queue (events are dropped when full; a
/// later event re-triggers the sync) and on the paths coalesced into one batch.
/// Together they keep memory bounded under heavy filesystem churn.
const WATCH_CHANNEL_CAP: usize = 1024;
const WATCH_MAX_BATCH_PATHS: usize = 4096;

// ---------------------------------------------------------------------------
// Rule tracking
// ---------------------------------------------------------------------------

struct RuleEntry {
	service_name: String,
	container_name: String,
	rule: WatchRule,
	abs_path: PathBuf,
	/// Absolute path of the service's local build context, when one exists.
	/// `None` for services without a local `build:` (image-based, or a remote
	/// context like `git://`/`https://`); `Some(...)` is the directory the
	/// `.dockerignore` is loaded from. The watch event's relative path is
	/// stripped against this when the build-context ignore file applies.
	build_context_abs: Option<PathBuf>,
	/// Patterns read once at watch start from the build context's
	/// `.dockerignore` / `.containerignore`. Empty when there is no build
	/// context or when the ignore file is missing. Each pattern is matched
	/// against the path relative to the build context, not to the rule's
	/// `path`, because that is what a `.dockerignore` is written against.
	build_context_patterns: Vec<String>,
	/// The service's effective mounts (bind sources + targets + read-only
	/// flags), shared across every rule of the same service. The per-path
	/// write-back check runs against this vector.
	mounts: Arc<Vec<EffectiveMount>>,
}

// ---------------------------------------------------------------------------
// Public watch command
// ---------------------------------------------------------------------------

impl Engine {
	/// Set up filesystem watchers from `develop.watch` rules and dispatch sync/rebuild/restart/exec actions on file changes.
	///
	/// **The session's exit status does not reflect the actions it ran.** A sync,
	/// rebuild, restart or exec that fails is warned about and the loop
	/// continues, so this returns `Ok(())` unless the watchers cannot be set up
	/// at all. That is deliberate and matches `docker compose watch`: a
	/// long-running developer loop should survive one failed rebuild rather than
	/// exit. See the exit-status section of `docs/commands.md`.
	pub async fn watch(&self, file: &ComposeFile) -> Result<()> {
		let mut rule_entries: Vec<RuleEntry> = Vec::new();

		// Pre-compute the effective mounts per service. One `EffectiveMount`
		// vector is enough for every rule of the service; sharing it via `Arc`
		// keeps the per-rule memory down and avoids re-canonicalising the same
		// bind sources per rule.
		let mut service_mounts: HashMap<String, Arc<Vec<EffectiveMount>>> = HashMap::new();
		for (name, service) in &file.services {
			service_mounts
				.entry(name.clone())
				.or_insert_with(|| Arc::new(effective_mounts(service, &self.base_dir)));
		}

		for (name, service) in &file.services {
			if let Some(dev) = &service.develop {
				for rule in &dev.watch {
					validate_sync_target(rule)?;
					// Canonicalise the joined absolute path so the rule's
					// literal `./` does not leak into the warn/info text
					// (`/tmp/d5/./src/f.txt`). The watcher accepts either
					// shape; the printed string is the only thing that needs
					// normalising here. Falls back to the join on canonicalize
					// failure (e.g. a not-yet-existing single-file rule), so
					// the watcher can still set up.
					let joined = self.base_dir.join(&rule.path);
					let abs = std::fs::canonicalize(&joined).unwrap_or(joined);

					// A service with a local `build:` carries a `.dockerignore`
					// the rule should pick up as implicit ignore content, per
					// the Compose Spec. Load it once per rule here so the
					// per-event evaluation is just a pattern match, not a
					// file read on every change.
					let (build_context_abs, build_context_patterns) =
						ignore_filter::local_build_context_patterns(&self.base_dir, service);
					let mounts = service_mounts
						.get(name)
						.cloned()
						.unwrap_or_else(|| Arc::new(Vec::new()));
					rule_entries.push(RuleEntry {
						service_name: name.clone(),
						container_name: self.first_replica_name(name, service),
						rule: rule.clone(),
						abs_path: abs,
						build_context_abs,
						build_context_patterns,
						mounts,
					});
				}
			}
		}

		if rule_entries.is_empty() {
			return Err(ComposeError::Watch(
				"no develop.watch rules configured".into(),
			));
		}

		// Startup warning: a sync rule whose root would write back through one
		// of the service's bind mounts. The copy itself is dropped per-path by
		// `writes_back` (the file is already shared), but the operator should
		// know up front so a missing copy on a fresh start is not mysterious.
		// The warning is once per rule, not per event.
		for entry in &rule_entries {
			let Some(target) = &entry.rule.target else {
				continue;
			};
			let container_path = join_container_path(&plan_sync_placement(
				&entry.abs_path,
				&entry.abs_path,
				target,
			));
			if let Some(host) = writes_back(&entry.mounts, &container_path, &entry.abs_path) {
				let ending = match entry.rule.action {
					WatchAction::SyncAndRestart => "; the restart still runs",
					WatchAction::SyncAndExec => "; the exec still runs",
					_ => "",
				};
				warn!(
					"{service}: sync target {target} maps back into the watched path ({host}) through a bind mount; \
					 copies that would land there are skipped so they cannot trigger themselves, \
					 and paths under a more specific mount still sync{ending}",
					service = entry.service_name,
					host = host.display(),
				);
			}
		}

		// Track which (container, dest) directories have been ensured so the
		// best-effort `mkdir -p` exec runs once per target rather than per event.
		let mut ensured: HashSet<(String, String)> = HashSet::new();

		// Warn once per (service, target) when a sync target sits on a
		// `read_only: true` root filesystem with no volume or tmpfs covering it:
		// every sync to it returns `read-only file system`, so the user should
		// learn at startup rather than after the first edit (#1897).
		let mut read_only_warned: HashSet<(String, String)> = HashSet::new();
		for entry in &rule_entries {
			let Some(target) = &entry.rule.target else {
				continue;
			};
			let Some(service) = file.services.get(&entry.service_name) else {
				continue;
			};
			let Some(msg) = read_only_target_warning(&entry.service_name, service, target) else {
				continue;
			};
			if !read_only_warned.insert((entry.service_name.clone(), target.clone())) {
				continue;
			}
			warn!("{msg}");
		}

		// Bounded channel: under heavy filesystem churn an unbounded queue (and the
		// per-batch path accumulation below) can grow without limit. Drop events
		// when the buffer is full: a later event re-triggers the sync, so no state
		// is permanently lost, but memory stays bounded.
		let (tx, mut rx) = mpsc::channel::<notify::Result<notify::Event>>(WATCH_CHANNEL_CAP);
		// Overflow flag: `events::enqueue` sets this on `TrySendError::Full` so the
		// event loop can spot a dropped event and run a full resync to recover
		// any state (e.g. a deletion) the dropped event carried. The flag is
		// `Arc<AtomicBool>` because the notify callback owns one clone and the
		// watch loop the other; `SeqCst` keeps the "did a drop happen" decision
		// and the loop's `swap(false)` to clear it in a single happens-before.
		let overflow = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
		// Wakeup sidekick for the flag. Without it, the loop can drain the
		// channel, pass both recovery checks, and block in `recv` between an
		// `enqueue` that saw `Full` and the same call's flag store; the flag
		// would then sit set with nothing to wake the loop. `notify_one` keeps
		// a permit when nobody is waiting, so the wakeup cannot be lost.
		let overflow_wake = std::sync::Arc::new(Notify::new());
		let (overflow_cb, overflow_wake_cb) = (overflow.clone(), overflow_wake.clone());
		let mut watcher = RecommendedWatcher::new(
			move |res| events::enqueue(&tx, &overflow_cb, &overflow_wake_cb, res),
			notify::Config::default(),
		)
		.map_err(|e| ComposeError::Watch(e.to_string()))?;

		// Register the watcher BEFORE the initial sync runs. A change written
		// between the initial sync and the registration would otherwise never
		// reach the container, because the initial sync's own reads complete
		// before the watcher is listening. This is safe in combination with
		// the upstream filter: `events::enqueue` drops Access events at the
		// callback boundary, so the initial sync's reads never reach the
		// channel and cannot push out real edits (#1984).
		for entry in &rule_entries {
			if entry.abs_path.exists() {
				watcher
					.watch(&entry.abs_path, RecursiveMode::Recursive)
					.map_err(|e| ComposeError::Watch(e.to_string()))?;
			} else {
				warn!("watch path not found: {}", entry.abs_path.display());
			}
		}

		self.sync_all(&rule_entries, &mut ensured, "initial sync")
			.await;

		info!("watching {} rule(s); Ctrl+C to stop", rule_entries.len());

		let debounce = Duration::from_millis(100);
		// Track which (service, pattern) tuples already triggered a legacy
		// warning. The warning is informational, not per-event: a pattern
		// that matches a hundred files across a long session should produce
		// one warning, not a hundred.
		let mut legacy_ignored_warned: HashSet<(String, String)> = HashSet::new();
		let mut legacy_included_warned: HashSet<(String, String)> = HashSet::new();

		loop {
			// Overflow check has to run before the `recv` so a flag set by an
			// `enqueue` racing this iteration does not sit until the next
			// event; it runs again after dispatch for the same reason.
			self.recover_from_overflow(&overflow, &rule_entries, &mut ensured)
				.await;

			let event = tokio::select! {
				ev = rx.recv() => match ev {
					Some(Ok(e)) => e,
					Some(Err(e)) => { warn!("notify error: {e}"); continue; }
					None => break,
				},
				_ = overflow_wake.notified() => continue,
				_ = tokio::signal::ctrl_c() => break,
			};

			// The channel is filtered upstream by `events::enqueue`, so only
			// Create/Modify/Remove events land here. The check below is kept
			// as defence in depth: a future change to the filter would
			// otherwise silently start re-feeding Access events into the
			// dispatch loop and re-open the read-triggered feedback cycle.
			if !is_dispatch_event(&event.kind) {
				continue;
			}

			let mut paths = event.paths;
			let deadline = tokio::time::Instant::now() + debounce;
			// Coalesce events within the debounce window, but stop accumulating once
			// the batch is large so a burst of churn cannot grow `paths` without
			// bound; the remaining events fall into the next batch.
			while paths.len() < WATCH_MAX_BATCH_PATHS {
				match tokio::time::timeout_at(deadline, rx.recv()).await {
					Ok(Some(Ok(e))) => {
						if is_dispatch_event(&e.kind) {
							paths.extend(e.paths);
						}
					}
					_ => break,
				}
			}

			// Collapse paths that notify reported more than once inside the
			// debounce window (e.g. a write and a remove of the same file
			// arriving as a Create and a Remove within 100 ms). The dispatch
			// makes its upload/remove decision per path from the host, so
			// the only thing the second occurrence of a path would do is
			// race the first one; keeping the first-seen order is enough
			// to make that race deterministic.
			let mut seen: HashSet<PathBuf> = HashSet::with_capacity(paths.len());
			paths.retain(|p| seen.insert(p.clone()));

			// A debounce batch may hold many files that map to the same whole-
			// container action; rebuild/restart each container at most once per
			// batch. Sync-type actions stay per-file (each changed file is synced).
			let mut done: std::collections::HashSet<(u8, String)> =
				std::collections::HashSet::new();

			'outer: for path in &paths {
				for entry in &rule_entries {
					if !path.starts_with(&entry.abs_path) {
						continue;
					}

					let mut ctx = ignore_filter::RuleContext {
						rule_abs: &entry.abs_path,
						ctx_abs: entry.build_context_abs.as_deref(),
						base_dir: &self.base_dir,
						service_name: &entry.service_name,
						rule_path: &entry.rule.path,
						ctx_patterns: &entry.build_context_patterns,
						warned: &mut legacy_ignored_warned,
					};
					if ignore_filter::ignored_with_fallback(path, &entry.rule.ignore, &mut ctx) {
						continue;
					}
					ctx.warned = &mut legacy_included_warned;
					if !ignore_filter::included_with_fallback(path, &entry.rule.include, &mut ctx) {
						continue;
					}

					// Collapse repeated rebuild/restart of the same container within
					// this batch into one; the action's effect is whole-container, so
					// a second run is pure waste.
					let dedup_key = match &entry.rule.action {
						WatchAction::Rebuild => Some((0, entry.service_name.clone())),
						WatchAction::Restart => Some((1, entry.container_name.clone())),
						_ => None,
					};
					if let Some(key) = dedup_key {
						if !done.insert(key) {
							continue 'outer;
						}
					}

					debug!("dispatch {:?} for {}", entry.rule.action, path.display());

					if let Err(e) = self.dispatch_action(file, path, entry, &mut ensured).await {
						warn!("watch action failed: {e}");
					}

					continue 'outer;
				}
			}

			// The recovery check ran before this batch started; an overflow
			// signal that came in while the batch was being dispatched
			// would otherwise wait out the next `recv`. The helper is a
			// no-op when the flag is false.
			self.recover_from_overflow(&overflow, &rule_entries, &mut ensured)
				.await;
		}

		Ok(())
	}

	async fn dispatch_action(
		&self,
		file: &ComposeFile,
		path: &Path,
		entry: &RuleEntry,
		ensured: &mut HashSet<(String, String)>,
	) -> Result<()> {
		match &entry.rule.action {
			WatchAction::Sync => {
				self.maybe_sync(entry, path, ensured).await?;
			}
			WatchAction::Rebuild => {
				self.watch_rebuild(file, &entry.service_name).await?;
			}
			WatchAction::Restart => {
				self.watch_restart(&entry.container_name).await?;
			}
			WatchAction::SyncAndRestart => {
				self.maybe_sync(entry, path, ensured).await?;
				self.watch_restart(&entry.container_name).await?;
			}
			WatchAction::SyncAndExec => {
				self.maybe_sync(entry, path, ensured).await?;
				if let Some(exec) = &entry.rule.exec {
					self.watch_exec(&entry.container_name, exec.command.clone())
						.await?;
				}
			}
		}
		Ok(())
	}
}

#[cfg(test)]
#[path = "watch_tests.rs"]
mod watch_tests;
