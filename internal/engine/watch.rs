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

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::libpod::types::exec::{ExecCreateConfig, ExecCreateResponse, ExecStartConfig};
use crate::libpod::{urlencoded, LogOutput};
use bytes::Bytes;
use flate2::write::GzEncoder;
use flate2::Compression;
use futures::StreamExt;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::compose::types::{ComposeFile, WatchAction, WatchRule};
use crate::error::{ComposeError, Result};

use super::Engine;

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

		let (tx, mut rx) = mpsc::unbounded_channel::<notify::Result<notify::Event>>();
		let mut watcher = RecommendedWatcher::new(
			move |res| {
				let _ = tx.send(res);
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
			while let Ok(Some(Ok(e))) = tokio::time::timeout_at(deadline, rx.recv()).await {
				paths.extend(e.paths);
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

		let path = format!(
			"/libpod/containers/{}/archive?path={}",
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
		self.build_service(service_name, service).await?;
		let container_name = self.container_name(service_name, service);
		self.create_and_start(&container_name, service_name, service, file)
			.await
	}

	async fn watch_restart(&self, container_name: &str) -> Result<()> {
		info!("restarting {container_name}");
		let stop_path = format!("/libpod/containers/{}/stop?t=5", urlencoded(container_name));
		let _ = self.client.post_empty_ok(&stop_path).await;
		let start_path = format!("/libpod/containers/{}/start", urlencoded(container_name));
		self.client.post_empty_ok(&start_path).await.map_err(ComposeError::Podman)?;
		Ok(())
	}

	async fn watch_exec(&self, container_name: &str, cmd: Vec<String>) -> Result<()> {
		let exec_cfg = ExecCreateConfig {
			cmd: Some(cmd),
			attach_stdout: Some(true),
			attach_stderr: Some(true),
			..Default::default()
		};
		let create_path = format!("/libpod/containers/{}/exec", urlencoded(container_name));
		let resp: ExecCreateResponse = self
			.client
			.post_json(&create_path, &exec_cfg)
			.await
			.map_err(ComposeError::Podman)?;

		let start_cfg = ExecStartConfig { detach: false, tty: false };
		let start_path = format!("/libpod/exec/{}/start", urlencoded(&resp.id));
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
// Tar builder for sync
// ---------------------------------------------------------------------------

fn build_sync_tar(src: &Path) -> Result<Vec<u8>> {
	let encoder = GzEncoder::new(Vec::new(), Compression::default());
	let mut tar = tar::Builder::new(encoder);

	if src.is_dir() {
		for abs in super::walk_dir(src).map_err(ComposeError::Io)? {
			let rel = abs
				.strip_prefix(src)
				.map_err(|_| ComposeError::Build("path strip".into()))?;
			if abs.is_dir() {
				tar.append_dir(rel, &abs)
					.map_err(|e| ComposeError::Build(e.to_string()))?;
			} else {
				tar.append_path_with_name(&abs, rel)
					.map_err(|e| ComposeError::Build(e.to_string()))?;
			}
		}
	} else if let Some(name) = src.file_name() {
		tar.append_path_with_name(src, name)
			.map_err(|e| ComposeError::Build(e.to_string()))?;
	}

	let gz = tar
		.into_inner()
		.map_err(|e| ComposeError::Build(e.to_string()))?;
	let bytes = gz
		.finish()
		.map_err(|e| ComposeError::Build(e.to_string()))?;
	Ok(bytes)
}

// ---------------------------------------------------------------------------
// Filter helpers
// ---------------------------------------------------------------------------

fn is_ignored(path: &str, patterns: &[String]) -> bool {
	for pat in patterns {
		if pat.ends_with('/') {
			if path.starts_with(pat.as_str()) {
				return true;
			}
		} else if path == pat.as_str()
			|| (path.starts_with(pat.as_str()) && path.as_bytes().get(pat.len()) == Some(&b'/'))
		{
			return true;
		}
	}
	false
}

fn is_included(path: &str, patterns: &[String]) -> bool {
	for pat in patterns {
		if pat.starts_with("*.") {
			let ext = &pat[1..];
			if path.ends_with(ext) {
				return true;
			}
		} else if pat.ends_with('/') {
			if path.starts_with(pat.as_str()) {
				return true;
			}
		} else if path == pat.as_str()
			|| (path.len() > pat.len() + 1
				&& path.as_bytes()[path.len() - pat.len() - 1] == b'/'
				&& path.ends_with(pat.as_str()))
		{
			return true;
		}
	}
	false
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
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::{build_sync_tar, is_ignored, is_included};
	use std::fs;
	use tempfile::tempdir;

	fn pats(v: &[&str]) -> Vec<String> {
		v.iter().map(|s| s.to_string()).collect()
	}

	// is_ignored -----------------------------------------------------------

	#[test]
	fn ignored_exact_file() {
		assert!(is_ignored("Makefile", &pats(&["Makefile"])));
	}

	#[test]
	fn ignored_not_prefix_match() {
		assert!(!is_ignored("Makefile.local", &pats(&["Makefile"])));
	}

	#[test]
	fn ignored_dir_prefix() {
		assert!(is_ignored("node_modules/foo.js", &pats(&["node_modules/"])));
	}

	#[test]
	fn ignored_dir_prefix_no_partial() {
		assert!(!is_ignored("nonode_modules/foo", &pats(&["node_modules/"])));
	}

	#[test]
	fn ignored_path_with_slash() {
		assert!(is_ignored("vendor/lib.rs", &pats(&["vendor"])));
	}

	#[test]
	fn ignored_empty_patterns() {
		assert!(!is_ignored("anything.rs", &[]));
	}

	#[test]
	fn ignored_no_match() {
		assert!(!is_ignored("src/main.rs", &pats(&["target/", "*.log"])));
	}

	// is_included ----------------------------------------------------------

	#[test]
	fn included_glob_extension() {
		assert!(is_included("src/main.rs", &pats(&["*.rs"])));
	}

	#[test]
	fn included_glob_no_match() {
		assert!(!is_included("src/main.go", &pats(&["*.rs"])));
	}

	#[test]
	fn included_dir_prefix() {
		assert!(is_included("src/main.rs", &pats(&["src/"])));
	}

	#[test]
	fn included_dir_prefix_no_match() {
		assert!(!is_included("test/main.rs", &pats(&["src/"])));
	}

	#[test]
	fn included_exact_match() {
		assert!(is_included("Makefile", &pats(&["Makefile"])));
	}

	#[test]
	fn included_path_segment_suffix() {
		assert!(is_included("src/lib.rs", &pats(&["lib.rs"])));
	}

	#[test]
	fn included_empty_patterns() {
		assert!(!is_included("anything", &[]));
	}

	// build_sync_tar -------------------------------------------------------

	#[test]
	fn sync_tar_single_file() {
		let dir = tempdir().unwrap();
		let file = dir.path().join("hello.txt");
		fs::write(&file, b"hello world").unwrap();
		let bytes = build_sync_tar(&file).unwrap();
		// gzip magic bytes
		assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
	}

	#[test]
	fn sync_tar_directory() {
		let dir = tempdir().unwrap();
		fs::write(dir.path().join("a.txt"), b"file a").unwrap();
		fs::create_dir(dir.path().join("sub")).unwrap();
		fs::write(dir.path().join("sub/b.txt"), b"file b").unwrap();
		let bytes = build_sync_tar(dir.path()).unwrap();
		assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
	}

	#[test]
	fn sync_tar_path_with_no_file_name() {
		// A path that has no file_name (e.g. root "/") — tar should be empty but valid.
		let dir = tempdir().unwrap();
		// Empty directory — no entries other than root
		let bytes = build_sync_tar(dir.path()).unwrap();
		assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
	}
}
