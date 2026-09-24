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

mod placement;
pub(in crate::engine) mod sync;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::libpod::types::exec::{
	ExecCreateConfig, ExecCreateResponse, ExecInspect, ExecStartConfig,
};
use crate::libpod::{urlencoded, LogOutput, API_PREFIX};
use futures_util::StreamExt;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::compose::types::{ComposeFile, WatchAction, WatchRule};
use crate::error::{ComposeError, Result};

use placement::{
	is_dispatch_event, is_remove_event, join_container_path, mark_dir_ensured, mkdir_p_argv,
	plan_remove_placement, plan_sync_placement, target_is_on_a_mount, validate_sync_target,
	SyncPlacement,
};
use sync::{is_ignored, is_included};

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
	///
	/// **The session's exit status does not reflect the actions it ran.** A sync,
	/// rebuild, restart or exec that fails is warned about and the loop
	/// continues, so this returns `Ok(())` unless the watchers cannot be set up
	/// at all. That is deliberate and matches `docker compose watch`: a
	/// long-running developer loop should survive one failed rebuild rather than
	/// exit. Unlike every other command here, it means a caller cannot gate on
	/// the status; see the exit-status section of `docs/commands.md`.
	pub async fn watch(&self, file: &ComposeFile) -> Result<()> {
		let mut rule_entries: Vec<RuleEntry> = Vec::new();

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
					rule_entries.push(RuleEntry {
						service_name: name.clone(),
						container_name: self.first_replica_name(name, service),
						rule: rule.clone(),
						abs_path: abs,
					});
				}
			}
		}

		if rule_entries.is_empty() {
			// docker compose watch errors when nothing is configured; match that
			// instead of silently exiting 0.
			return Err(ComposeError::Watch(
				"no develop.watch rules configured".into(),
			));
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
			if service.read_only != Some(true) {
				continue;
			}
			if !read_only_warned.insert((entry.service_name.clone(), target.clone())) {
				continue;
			}
			let mut mounts: Vec<&str> = service.volumes.iter().map(|v| v.target()).collect();
			let tmpfs_paths: Vec<String> = service
				.tmpfs
				.to_list()
				.into_iter()
				.map(|s| s.split(':').next().unwrap_or("").to_string())
				.collect();
			mounts.extend(tmpfs_paths.iter().map(String::as_str));
			if !target_is_on_a_mount(target, &mounts) {
				warn!(
					"{}: sync target {} is on the read-only root filesystem (read_only: true) and no volume or tmpfs covers it, so every sync to it will fail; mount a volume or tmpfs at {}",
					entry.service_name, target, target
				);
			}
		}

		for entry in &rule_entries {
			if entry.rule.initial_sync {
				// A missing watch path cannot be synced: the existence check
				// in the watcher-setup loop below will warn about it, so skip
				// both the `initial sync` log and the (doomed) sync itself.
				if !entry.abs_path.exists() {
					continue;
				}
				if let Some(target) = &entry.rule.target {
					info!("initial sync {} -> {target}", entry.abs_path.display());
					if let Err(e) = self
						.sync_to_container(
							&entry.container_name,
							&entry.abs_path,
							&entry.abs_path,
							target,
							&mut ensured,
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
		// when the buffer is full: a later event re-triggers the sync, so no state
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

		info!("watching {} rule(s); Ctrl+C to stop", rule_entries.len());

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

			// Ignore Access/Other events: only create/modify/remove/rename drive a
			// sync, matching docker compose and avoiding the read-triggered
			// self-feedback loop.
			if !is_dispatch_event(&event.kind) {
				continue;
			}

			let mut paths = event.paths;
			let event_kind = event.kind;
			let deadline = tokio::time::Instant::now() + debounce;
			// Coalesce events within the debounce window, but stop accumulating once
			// the batch is large so a burst of churn cannot grow `paths` without
			// bound; the remaining events fall into the next batch.
			while paths.len() < WATCH_MAX_BATCH_PATHS {
				match tokio::time::timeout_at(deadline, rx.recv()).await {
					Ok(Some(Ok(e))) => {
						if is_dispatch_event(&e.kind) {
							// A single notify `Event` carries one `kind` across all
							// its paths. The first event's kind owns the batch: a
							// later event of a different kind in the same debounce
							// window is rare (notify coalesces by file), and treating
							// it as the dominant kind keeps the dispatch's contract
							// ("this path was removed / this path was changed") the
							// same as a single-event loop would. Paths are accumulated
							// either way: the only thing the dominant kind affects is
							// whether the dispatch later treats the batch as an upload
							// or a removal.
							paths.extend(e.paths);
						}
					}
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

					if let Err(e) = self
						.dispatch_action(file, path, &event_kind, entry, &mut ensured)
						.await
					{
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
		event_kind: &notify::EventKind,
		entry: &RuleEntry,
		ensured: &mut HashSet<(String, String)>,
	) -> Result<()> {
		match &entry.rule.action {
			WatchAction::Sync => {
				if let Some(target) = &entry.rule.target {
					self.dispatch_sync(
						&entry.container_name,
						&entry.abs_path,
						path,
						target,
						event_kind,
						ensured,
					)
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
					self.dispatch_sync(
						&entry.container_name,
						&entry.abs_path,
						path,
						target,
						event_kind,
						ensured,
					)
					.await?;
				}
				self.watch_restart(&entry.container_name).await?;
			}
			WatchAction::SyncAndExec => {
				if let Some(target) = &entry.rule.target {
					self.dispatch_sync(
						&entry.container_name,
						&entry.abs_path,
						path,
						target,
						event_kind,
						ensured,
					)
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

	/// Pick the right sync dispatch for the event kind: copy on a change, remove
	/// on a deletion. A rebuild/restart/exec rule that does not sync does not
	/// reach this; sync-family actions all funnel through here so the
	/// removal path is owned in one place.
	async fn dispatch_sync(
		&self,
		container: &str,
		root: &Path,
		changed: &Path,
		target: &str,
		event_kind: &notify::EventKind,
		ensured: &mut HashSet<(String, String)>,
	) -> Result<()> {
		if is_remove_event(event_kind) {
			self.remove_from_container(container, root, changed, target)
				.await
		} else {
			self.sync_to_container(container, root, changed, target, ensured)
				.await
		}
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
		ensured: &mut HashSet<(String, String)>,
	) -> Result<()> {
		let SyncPlacement {
			entry_name,
			dest_dir,
		} = plan_sync_placement(root, changed, target);
		// `build_sync_tar_stream` walks the changed directory (possibly many
		// entries) and gzips it inside a `spawn_blocking` task that pipes the
		// bytes through a bounded channel to the PUT body. On an async
		// runtime, doing that on the calling task would block the executor
		// for the duration; a multi-megabyte tree turns a one-line edit into
		// a freeze that the rest of `watch`'s I/O cannot escape. Same shape
		// as the `cp` packer; only the wrapper (gzip) and the walk
		// (`sync_walk`) differ. The recorded entry list rides in
		// `PackedStream.producer` and is what `put_archive_verified` reads
		// back to confirm the upload landed.
		let packed = super::copy::build_sync_tar_stream_for_watch(
			changed,
			Path::new(&entry_name),
			crate::engine::copy::CpByteCounter::new().inner().clone(),
		);

		// docker compose watch creates the sync target directory when it is
		// missing; match that so a sync to a not-yet-existing path works instead
		// of failing the archive PUT. Best-effort: if mkdir is unavailable the
		// PUT below still surfaces the real error. Run it once per (container,
		// dest) so repeated syncs to the same target skip the redundant exec.
		if mark_dir_ensured(ensured, container, &dest_dir) {
			let _ = self.watch_exec(container, mkdir_p_argv(&dest_dir)).await;
		}

		// Shared with `cp`: the same archive upload, and the same #1097 Podman-6
		// apply-then-close handling (the endpoint applies the tar then closes
		// without a response; the upload is confirmed by the entry matching what
		// was sent). `watch` used to have its own copy of this PUT, which is how
		// the two drifted apart and left sync unfixed on Podman 6.
		let uploaded_kind = crate::engine::copy::uploaded_entry_kind(changed, false);
		self.put_archive_verified(container, &dest_dir, &entry_name, packed, uploaded_kind)
			.await?;

		info!("synced {} -> {target}", changed.display());
		Ok(())
	}

	/// Mirror a host-side removal into the container.
	///
	/// Bounded: the path inside the container is computed from the rule's
	/// `path` and `target` only (no caller-supplied path), and only entries
	/// inside the target directory are issued. A removal of the target
	/// directory itself is refused; that is a different operation (the entire
	/// mapped area), not an entry delete, and would otherwise `rm -rf` the
	/// destination.
	///
	/// The container-side deletion is a `rm -rf` exec rather than a DELETE
	/// against the archive endpoint. libpod's archive DELETE was answered
	/// with `405 Method Not Allowed` on Podman 5.7.0 (the endpoint documents
	/// only GET/PUT/HEAD), and the engine contract here is "the file is
	/// gone inside the container" — both paths satisfy it, and the exec
	/// path works on every libpod version that has the exec endpoint.
	/// Matching docker compose's own watch handler (#17 in their tar syncer
	/// upstream), which uses `rm -rf` for the same reason.
	async fn remove_from_container(
		&self,
		container: &str,
		root: &Path,
		removed: &Path,
		target: &str,
	) -> Result<()> {
		let placement = plan_remove_placement(root, removed, target);
		let container_path = join_container_path(&placement);
		if container_path == "/" || container_path.trim() == placement.dest_dir.trim() {
			// Refuse to delete the rule's own target directory; see the
			// function-level note. A single-file rule's `dest_dir` is the
			// parent of the entry (e.g. `/app`), so this guard fires when
			// the rule target is a bare directory and the changed path is
			// the directory itself, which is what the function-level note
			// describes.
			return Err(ComposeError::Watch(format!(
				"sync: refusing to remove the rule target directory {container_path}"
			)));
		}
		// `rm -f` (not `rm -rf`): the path is bounded by the rule's
		// `path`/`target` so it cannot reach outside the destination, and
		// refusing to recurse keeps the operation scoped to the single
		// removed entry. `rm -f` swallows the missing-file case (the file
		// was already gone inside the container, which is fine).
		let argv = vec![
			"rm".to_string(),
			"-f".to_string(),
			"--".to_string(),
			container_path.clone(),
		];
		self.watch_exec(container, argv)
			.await
			.map_err(|e| ComposeError::Watch(format!("sync: remove {container_path}: {e}")))?;
		info!(
			"removed {container_path} from {container} ({})",
			removed.display()
		);
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
		self.create_project_secrets(file).await?;
		let container_name = self.first_replica_name(service_name, service);
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

	/// Run `cmd` inside `container_name` via libpod's exec endpoint, streaming its
	/// output to the current process's stdout/stderr.
	///
	/// A non-zero exit code is returned as `Err(ComposeError::Watch(...))` so a
	/// failed container-side step (e.g. `rm -f` denied) is not silently logged
	/// as done (#1897). Callers that want best-effort behaviour (the
	/// per-target `mkdir -p`) can ignore the result; others propagate it and
	/// the watch event loop logs `watch action failed` without aborting the
	/// session.
	async fn watch_exec(&self, container_name: &str, cmd: Vec<String>) -> Result<()> {
		// Keep the joined argv before `cmd` moves into `ExecCreateConfig`, so a
		// non-zero exit can quote it in the error message.
		let cmd_display = cmd.join(" ");
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

		// The stream ending does not imply the process finished cleanly; read
		// the exit code so a denied `rm -f` (or any non-zero exit) surfaces as
		// an error rather than a successful no-op (#1897). The inspect request
		// matches `Engine::exec_hook`'s shape: GET exec/json → ExecInspect.
		let inspect_path = format!("{API_PREFIX}/exec/{}/json", urlencoded(&resp.id));
		let inspect: ExecInspect = self
			.client
			.get_json(&inspect_path)
			.await
			.map_err(ComposeError::Podman)?;
		if let Some(code) = inspect.exit_code {
			if code != 0 {
				return Err(ComposeError::Watch(format!(
					"`{cmd_display}` exited with status {code}"
				)));
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
		let mut ensured = HashSet::new();
		self.sync_to_container(container, src, src, target, &mut ensured)
			.await
	}

	/// Test seam: delete the entry `path` would have written under `target`
	/// from `container`. Mirrors the live `dispatch_action` path that runs on
	/// a `Remove` notify event.
	pub async fn test_remove_from_container(
		&self,
		container: &str,
		src: &Path,
		target: &str,
	) -> Result<()> {
		self.remove_from_container(container, src, src, target)
			.await
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

	/// The network aliases a container answers to, flattened across every
	/// network it is attached to.
	///
	/// The seam that lets a test check **podup's** contribution to service-name
	/// resolution (registering the compose service name as an alias) without
	/// depending on the runtime's DNS server being up to answer for it. Those
	/// are two layers, and a test that only measures the second blames podup for
	/// the first's failures (#1330).
	pub async fn test_container_aliases(&self, container: &str) -> Result<Vec<String>> {
		let path = format!(
			"{}/containers/{}/json",
			crate::libpod::API_PREFIX,
			crate::libpod::urlencoded(container)
		);
		let inspect: crate::libpod::types::container::ContainerInspect = self
			.client
			.get_json(&path)
			.await
			.map_err(crate::error::ComposeError::Podman)?;
		Ok(inspect
			.network_settings
			.map(|n| {
				n.networks
					.into_values()
					.flat_map(|a| a.aliases)
					.collect::<Vec<_>>()
			})
			.unwrap_or_default())
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
