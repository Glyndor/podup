//! File-watch engine for `develop: watch:` rules.
//!
//! [`Engine::watch`] sets up an `inotify`/`kqueue` watcher via `notify`, then
//! dispatches each change event to the matching [`WatchRule`]. Debouncing
//! collapses rapid bursts into a single action. Actions:
//! - `sync` — tar the changed file and upload it into the container
//! - `rebuild` — stop container, rebuild image, restart
//! - `restart` — stop and start the container without rebuilding
//! - `sync+restart` — sync first, then restart
//! - `sync+exec` — sync, then run the rule's `exec` command inside the container

mod sync;

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::libpod::types::exec::{ExecCreateConfig, ExecCreateResponse, ExecStartConfig};
use crate::libpod::{urlencoded, LogOutput, API_PREFIX};
use bytes::Bytes;
use futures_util::StreamExt;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::compose::types::{ComposeFile, WatchAction, WatchRule};
use crate::error::{ComposeError, Result};

use sync::{build_sync_tar, is_ignored, is_included};

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
}

/// Where a changed host path lands inside the container for a `sync` action:
/// the archive entry name and the directory the tar is extracted at.
struct SyncPlacement {
	/// Archive path the changed entry occupies inside the tar.
	entry_name: String,
	/// Container directory the archive is PUT (extracted) at.
	dest_dir: String,
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
fn plan_sync_placement(root: &Path, changed: &Path, target: &str) -> SyncPlacement {
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

// ---------------------------------------------------------------------------
// Public watch command
// ---------------------------------------------------------------------------

impl Engine {
	/// Set up filesystem watchers from `develop.watch` rules and dispatch sync/rebuild/restart/exec actions on file changes.
	pub async fn watch(&self, file: &ComposeFile) -> Result<()> {
		let mut rule_entries: Vec<RuleEntry> = Vec::new();

		for (name, service) in &file.services {
			if let Some(dev) = &service.develop {
				for rule in &dev.watch {
					let abs = self.base_dir.join(&rule.path);
					rule_entries.push(RuleEntry {
						service_name: name.clone(),
						container_name: self.container_name(name, service),
						rule: rule.clone(),
						abs_path: abs,
					});
				}
			}
		}

		if rule_entries.is_empty() {
			info!("no develop.watch rules found");
			return Ok(());
		}

		for entry in &rule_entries {
			if entry.rule.initial_sync {
				if let Some(target) = &entry.rule.target {
					info!("initial sync {} -> {target}", entry.abs_path.display());
					if let Err(e) = self
						.sync_to_container(
							&entry.container_name,
							&entry.abs_path,
							&entry.abs_path,
							target,
						)
						.await
					{
						warn!("initial sync failed: {e}");
					}
				}
			}
		}

		// Bounded channel: under heavy filesystem churn an unbounded queue (and the
		// per-batch path accumulation below) can grow without limit. Drop events
		// when the buffer is full — a later event re-triggers the sync, so no state
		// is permanently lost, but memory stays bounded.
		let (tx, mut rx) = mpsc::channel::<notify::Result<notify::Event>>(WATCH_CHANNEL_CAP);
		let mut watcher = RecommendedWatcher::new(
			move |res| {
				// A full bounded channel drops this event; a later event
				// re-triggers the sync, so no state is lost, but trace the drop
				// instead of swallowing it silently.
				if let Err(e) = tx.try_send(res) {
					debug!("watch event dropped (channel full or closed): {e}");
				}
			},
			notify::Config::default(),
		)
		.map_err(|e| ComposeError::Watch(e.to_string()))?;

		for entry in &rule_entries {
			if entry.abs_path.exists() {
				watcher
					.watch(&entry.abs_path, RecursiveMode::Recursive)
					.map_err(|e| ComposeError::Watch(e.to_string()))?;
			} else {
				warn!("watch path not found: {}", entry.abs_path.display());
			}
		}

		info!("watching {} rule(s) — Ctrl+C to stop", rule_entries.len());

		let debounce = Duration::from_millis(100);

		loop {
			let event = tokio::select! {
				ev = rx.recv() => match ev {
					Some(Ok(e)) => e,
					Some(Err(e)) => { warn!("notify error: {e}"); continue; }
					None => break,
				},
				_ = tokio::signal::ctrl_c() => break,
			};

			let mut paths = event.paths;
			let deadline = tokio::time::Instant::now() + debounce;
			// Coalesce events within the debounce window, but stop accumulating once
			// the batch is large so a burst of churn cannot grow `paths` without
			// bound; the remaining events fall into the next batch.
			while paths.len() < WATCH_MAX_BATCH_PATHS {
				match tokio::time::timeout_at(deadline, rx.recv()).await {
					Ok(Some(Ok(e))) => paths.extend(e.paths),
					_ => break,
				}
			}

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

					let rel = path.strip_prefix(&self.base_dir).unwrap_or(path.as_path());
					let rel_str = rel.to_string_lossy();

					if is_ignored(&rel_str, &entry.rule.ignore) {
						continue;
					}
					if !entry.rule.include.is_empty() && !is_included(&rel_str, &entry.rule.include)
					{
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

					if let Err(e) = self.dispatch_action(file, path, entry).await {
						warn!("watch action failed: {e}");
					}

					continue 'outer;
				}
			}
		}

		Ok(())
	}

	async fn dispatch_action(
		&self,
		file: &ComposeFile,
		path: &Path,
		entry: &RuleEntry,
	) -> Result<()> {
		match &entry.rule.action {
			WatchAction::Sync => {
				if let Some(target) = &entry.rule.target {
					self.sync_to_container(&entry.container_name, &entry.abs_path, path, target)
						.await?;
				}
			}
			WatchAction::Rebuild => {
				self.watch_rebuild(file, &entry.service_name).await?;
			}
			WatchAction::Restart => {
				self.watch_restart(&entry.container_name).await?;
			}
			WatchAction::SyncAndRestart => {
				if let Some(target) = &entry.rule.target {
					self.sync_to_container(&entry.container_name, &entry.abs_path, path, target)
						.await?;
				}
				self.watch_restart(&entry.container_name).await?;
			}
			WatchAction::SyncAndExec => {
				if let Some(target) = &entry.rule.target {
					self.sync_to_container(&entry.container_name, &entry.abs_path, path, target)
						.await?;
				}
				if let Some(exec) = &entry.rule.exec {
					self.watch_exec(&entry.container_name, exec.command.clone())
						.await?;
				}
			}
		}
		Ok(())
	}

	// -----------------------------------------------------------------------
	// Action helpers
	// -----------------------------------------------------------------------

	async fn sync_to_container(
		&self,
		container: &str,
		root: &Path,
		changed: &Path,
		target: &str,
	) -> Result<()> {
		let SyncPlacement {
			entry_name,
			dest_dir,
		} = plan_sync_placement(root, changed, target);
		let tar_bytes = build_sync_tar(changed, Path::new(&entry_name))?;

		// docker compose watch creates the sync target directory when it is
		// missing; match that so a sync to a not-yet-existing path works instead
		// of failing the archive PUT. Best-effort: if mkdir is unavailable the
		// PUT below still surfaces the real error.
		let _ = self
			.watch_exec(
				container,
				vec!["mkdir".into(), "-p".into(), dest_dir.clone()],
			)
			.await;

		let path = format!(
			"{API_PREFIX}/containers/{}/archive?path={}",
			urlencoded(container),
			urlencoded(&dest_dir),
		);
		self.client
			.put_bytes_ok(&path, Bytes::from(tar_bytes), "application/x-tar")
			.await
			.map_err(ComposeError::Podman)?;

		info!("synced {} -> {target}", changed.display());
		Ok(())
	}

	async fn watch_rebuild(&self, file: &ComposeFile, service_name: &str) -> Result<()> {
		let service = match file.services.get(service_name) {
			Some(s) => s,
			None => return Ok(()),
		};
		info!("rebuilding {service_name}");
		self.build_service(
			service_name,
			service,
			file,
			&crate::engine::BuildOptions::default(),
		)
		.await?;
		// Inline secrets/configs are materialised up front rather than in the
		// per-container path; ensure they exist before recreating the container.
		self.create_inline_secrets(file).await?;
		let container_name = self.container_name(service_name, service);
		self.create_and_start(&container_name, service_name, service, file, true)
			.await
	}

	async fn watch_restart(&self, container_name: &str) -> Result<()> {
		info!("restarting {container_name}");
		// Single atomic restart (no visible stopped window) instead of stop+start.
		let restart_path = format!(
			"{API_PREFIX}/containers/{}/restart?t=5",
			urlencoded(container_name)
		);
		self.client
			.post_empty_ok(&restart_path)
			.await
			.map_err(ComposeError::Podman)?;
		Ok(())
	}

	async fn watch_exec(&self, container_name: &str, cmd: Vec<String>) -> Result<()> {
		let exec_cfg = ExecCreateConfig {
			cmd: Some(cmd),
			attach_stdout: Some(true),
			attach_stderr: Some(true),
			..Default::default()
		};
		let create_path = format!(
			"{API_PREFIX}/containers/{}/exec",
			urlencoded(container_name)
		);
		let resp: ExecCreateResponse = self
			.client
			.post_json(&create_path, &exec_cfg)
			.await
			.map_err(ComposeError::Podman)?;

		let start_cfg = ExecStartConfig {
			detach: false,
			tty: false,
		};
		let start_path = format!("{API_PREFIX}/exec/{}/start", urlencoded(&resp.id));
		let start_resp = self
			.client
			.post_json_stream(&start_path, &start_cfg)
			.await
			.map_err(ComposeError::Podman)?;
		let mut stream = crate::libpod::parse_multiplexed(start_resp.into_body());

		while let Some(msg) = stream.next().await {
			match msg {
				Ok(LogOutput::StdOut { message }) => {
					print!("{}", String::from_utf8_lossy(&message));
				}
				Ok(LogOutput::StdErr { message }) => {
					eprint!("{}", String::from_utf8_lossy(&message));
				}
				Err(_) => break,
			}
		}
		Ok(())
	}
}
// ---------------------------------------------------------------------------
// Test helpers (feature-gated so they never appear in release builds)
// ---------------------------------------------------------------------------

#[cfg(feature = "test-helpers")]
impl Engine {
	/// Test seam: copy `src` into `container` at `target` via the watch sync
	/// path, treating `src` as both the watch-rule root and the changed entry
	/// (as the initial-sync path does).
	pub async fn test_sync_to_container(
		&self,
		container: &str,
		src: &Path,
		target: &str,
	) -> Result<()> {
		self.sync_to_container(container, src, src, target).await
	}

	/// Test seam: run the watch restart action against `container_name`.
	pub async fn test_watch_restart(&self, container_name: &str) -> Result<()> {
		self.watch_restart(container_name).await
	}

	/// Test seam: run the watch exec action (`cmd`) against `container_name`.
	pub async fn test_watch_exec(&self, container_name: &str, cmd: Vec<String>) -> Result<()> {
		self.watch_exec(container_name, cmd).await
	}

	/// All container names carrying this project's label (any state). Lets
	/// integration tests assert which service containers `run` did or did not
	/// create (e.g. that `--no-deps` skipped a dependency).
	pub async fn test_project_container_names(&self) -> Result<Vec<String>> {
		self.list_project_container_names(None).await
	}

	/// Run a command in the named container and return its captured stdout.
	///
	/// Integration tests use this to observe the effect of a watch action (e.g.
	/// that a synced file reached the container) and poll for it, instead of
	/// sleeping a fixed duration and assuming the action completed.
	pub async fn test_exec_capture(&self, container: &str, cmd: Vec<String>) -> Result<String> {
		let exec_cfg = ExecCreateConfig {
			cmd: Some(cmd),
			attach_stdout: Some(true),
			attach_stderr: Some(true),
			..Default::default()
		};
		let create_path = format!("{API_PREFIX}/containers/{}/exec", urlencoded(container));
		let resp: ExecCreateResponse = self
			.client
			.post_json(&create_path, &exec_cfg)
			.await
			.map_err(ComposeError::Podman)?;

		let start_cfg = ExecStartConfig {
			detach: false,
			tty: false,
		};
		let start_path = format!("{API_PREFIX}/exec/{}/start", urlencoded(&resp.id));
		let start_resp = self
			.client
			.post_json_stream(&start_path, &start_cfg)
			.await
			.map_err(ComposeError::Podman)?;
		let mut stream = crate::libpod::parse_multiplexed(start_resp.into_body());

		let mut out = String::new();
		while let Some(msg) = stream.next().await {
			if let LogOutput::StdOut { message } = msg.map_err(ComposeError::Podman)? {
				out.push_str(&String::from_utf8_lossy(&message));
			}
		}
		Ok(out)
	}
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::plan_sync_placement;
	use std::fs;
	use tempfile::tempdir;

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
