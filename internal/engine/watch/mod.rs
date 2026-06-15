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
						.sync_to_container(&entry.container_name, &entry.abs_path, target)
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
				let _ = tx.try_send(res);
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
					self.sync_to_container(&entry.container_name, path, target)
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
					self.sync_to_container(&entry.container_name, path, target)
						.await?;
				}
				self.watch_restart(&entry.container_name).await?;
			}
			WatchAction::SyncAndExec => {
				if let Some(target) = &entry.rule.target {
					self.sync_to_container(&entry.container_name, path, target)
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

	async fn sync_to_container(&self, container: &str, src: &Path, target: &str) -> Result<()> {
		let tar_bytes = build_sync_tar(src)?;

		let dest_dir = if target.ends_with('/') {
			target.to_string()
		} else {
			Path::new(target)
				.parent()
				.map(|p| p.to_string_lossy().into_owned())
				.filter(|s| !s.is_empty())
				.unwrap_or_else(|| "/".to_string())
		};

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

		info!("synced {} -> {target}", src.display());
		Ok(())
	}

	async fn watch_rebuild(&self, file: &ComposeFile, service_name: &str) -> Result<()> {
		let service = match file.services.get(service_name) {
			Some(s) => s,
			None => return Ok(()),
		};
		info!("rebuilding {service_name}");
		self.build_service(service_name, service, file).await?;
		let container_name = self.container_name(service_name, service);
		self.create_and_start(&container_name, service_name, service, file)
			.await
	}

	async fn watch_restart(&self, container_name: &str) -> Result<()> {
		info!("restarting {container_name}");
		let stop_path = format!(
			"{API_PREFIX}/containers/{}/stop?t=5",
			urlencoded(container_name)
		);
		if let Err(e) = self.client.post_empty_ok(&stop_path).await {
			tracing::debug!("stop before watch restart {container_name}: {e}");
		}
		let start_path = format!(
			"{API_PREFIX}/containers/{}/start",
			urlencoded(container_name)
		);
		self.client
			.post_empty_ok(&start_path)
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
	pub async fn test_sync_to_container(
		&self,
		container: &str,
		src: &Path,
		target: &str,
	) -> Result<()> {
		self.sync_to_container(container, src, target).await
	}

	pub async fn test_watch_restart(&self, container_name: &str) -> Result<()> {
		self.watch_restart(container_name).await
	}

	pub async fn test_watch_exec(&self, container_name: &str, cmd: Vec<String>) -> Result<()> {
		self.watch_exec(container_name, cmd).await
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
