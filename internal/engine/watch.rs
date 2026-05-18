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

use bollard::body_full;
use bollard::container::LogOutput;
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::query_parameters::{
    StartContainerOptions, StopContainerOptions, UploadToContainerOptions,
};
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

/// Pre-resolved watch rule: service identity + rule + absolute host path for fast matching.
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

        // Initial sync.
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

        // Notify watcher with tokio channel bridge.
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

            // Drain events within debounce window.
            let mut paths = event.paths;
            let deadline = tokio::time::Instant::now() + debounce;
            while let Ok(Some(Ok(e))) = tokio::time::timeout_at(deadline, rx.recv()).await {
                paths.extend(e.paths);
            }

            // Dispatch each changed path.
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

        self.docker
            .upload_to_container(
                container,
                Some(UploadToContainerOptions {
                    path: dest_dir,
                    no_overwrite_dir_non_dir: None,
                    copy_uidgid: None,
                }),
                body_full(Bytes::from(tar_bytes)),
            )
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
        let _ = self
            .docker
            .stop_container(
                container_name,
                Some(StopContainerOptions {
                    t: Some(5),
                    ..Default::default()
                }),
            )
            .await;
        self.docker
            .start_container(container_name, None::<StartContainerOptions>)
            .await?;
        Ok(())
    }

    async fn watch_exec(&self, container_name: &str, cmd: Vec<String>) -> Result<()> {
        let exec_id = self
            .docker
            .create_exec(
                container_name,
                CreateExecOptions::<String> {
                    cmd: Some(cmd),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    ..Default::default()
                },
            )
            .await?
            .id;

        match self.docker.start_exec(&exec_id, None).await? {
            StartExecResults::Attached { mut output, .. } => {
                while let Some(msg) = output.next().await {
                    match msg? {
                        LogOutput::StdOut { message } => {
                            print!("{}", String::from_utf8_lossy(&message));
                        }
                        LogOutput::StdErr { message } => {
                            eprint!("{}", String::from_utf8_lossy(&message));
                        }
                        _ => {}
                    }
                }
            }
            StartExecResults::Detached => {}
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
        for entry in walkdir::WalkDir::new(src).follow_links(false) {
            let entry = entry.map_err(|e| ComposeError::Io(e.into()))?;
            let abs = entry.path();
            let rel = abs
                .strip_prefix(src)
                .map_err(|_| ComposeError::Build("path strip".into()))?;
            if rel.as_os_str().is_empty() {
                continue;
            }
            if abs.is_dir() {
                tar.append_dir(rel, abs)
                    .map_err(|e| ComposeError::Build(e.to_string()))?;
            } else {
                tar.append_path_with_name(abs, rel)
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
