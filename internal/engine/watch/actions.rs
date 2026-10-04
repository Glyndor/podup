//! Per-event watch action helpers, extracted from `mod.rs` to keep that file
//! under the source-line limit.
//!
//! The dispatch loop in `super::mod::Engine::watch` calls these through
//! `dispatch_action`; they own the per-rule I/O (sync upload, sync remove,
//! rebuild, restart, exec). Pure placement decisions stay in
//! `super::placement`; the write-back loop belongs in `super::writeback`.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use futures_util::StreamExt;
use tracing::{debug, info};

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::types::exec::{
	ExecCreateConfig, ExecCreateResponse, ExecInspect, ExecStartConfig,
};
use crate::libpod::{urlencoded, LogOutput, API_PREFIX};

use crate::engine::copy;

use super::placement::{
	container_rel, join_container_path, mark_dir_ensured, mkdir_p_argv, plan_sync_placement,
	SyncPlacement,
};
use super::sync::has_permitted_entry;
use super::writeback::{writes_back, EffectiveMount};
use super::Engine;

impl Engine {
	#[allow(clippy::too_many_arguments)]
	pub(in crate::engine::watch) async fn sync_to_container(
		&self,
		container: &str,
		root: &Path,
		changed: &Path,
		target: &str,
		ensured: &mut HashSet<(String, String)>,
		mounts: &[EffectiveMount],
		watched_root: &Path,
	) -> Result<()> {
		let SyncPlacement {
			entry_name,
			dest_dir,
		} = plan_sync_placement(root, changed, target);
		// Per-entry filter: any descendant of this upload whose container
		// path lands in a deeper writable bind that maps back into the
		// watched tree is dropped, otherwise the copy would re-fire the
		// watcher. The packer still walks through skipped directories so a
		// safe deeper mount below a skipped one is still copied.
		let dest_dir_owned = dest_dir.clone();
		let mounts_owned: Arc<Vec<EffectiveMount>> = Arc::new(mounts.to_vec());
		let watched_root_owned = watched_root.to_path_buf();
		let skip: Arc<dyn Fn(&Path) -> bool + Send + Sync> = {
			let dest_dir = dest_dir_owned.clone();
			let mounts = Arc::clone(&mounts_owned);
			let watched_root = watched_root_owned.clone();
			Arc::new(move |name: &Path| {
				let entry = container_rel(name);
				let container_path = join_container_path(&SyncPlacement {
					dest_dir: dest_dir.clone(),
					entry_name: entry,
				});
				writes_back(&mounts, &container_path, &watched_root).is_some()
			})
		};
		// When the per-entry filter rejects every archive entry, the upload
		// would be a tar with no body, the verification step would fail,
		// and the failure would stop the restart or exec of a
		// `sync+restart` / `sync+exec` action. The `mkdir` below would
		// also fire and create a directory through the bind. Probe the
		// filter with the same rules `build_sync_tar` uses: if nothing
		// survives, log at `debug!` and return success so the surrounding
		// `sync+restart` / `sync+exec` still runs its other half.
		if !has_permitted_entry(changed, Path::new(&entry_name), &*skip)? {
			debug!(
				"nothing to copy for {changed}: every entry maps back into the watched path",
				changed = changed.display()
			);
			return Ok(());
		}

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
		let packed = copy::build_sync_tar_stream_for_watch(
			changed,
			Path::new(&entry_name),
			crate::engine::copy::CpByteCounter::new().inner().clone(),
			Arc::clone(&skip),
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
	/// `placement` is the resolved container placement the dispatch loop
	/// already computed: passing it in keeps the guard upstream (which
	/// short-circuits a removal whose container path maps back into the
	/// watched tree) and the `rm` here on the same container path, so the
	/// check and the operation cannot disagree.
	///
	/// The container-side deletion is a `rm -rf` exec rather than a DELETE
	/// against the archive endpoint. libpod's archive DELETE was answered
	/// with `405 Method Not Allowed` on Podman 5.7.0 (the endpoint documents
	/// only GET/PUT/HEAD), and the engine contract here is "the file is
	/// gone inside the container"; both paths satisfy it. Matching docker
	/// compose's own watch handler (#17 in their tar syncer upstream), which
	/// uses `rm -rf` for the same reason.
	pub(in crate::engine::watch) async fn remove_from_container(
		&self,
		container: &str,
		placement: SyncPlacement,
		removed: &Path,
	) -> Result<()> {
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

	pub(in crate::engine::watch) async fn watch_rebuild(
		&self,
		file: &ComposeFile,
		service_name: &str,
	) -> Result<()> {
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

	pub(in crate::engine::watch) async fn watch_restart(&self, container_name: &str) -> Result<()> {
		info!("restarting {container_name}");
		// Single atomic restart (no visible stopped window) instead of stop+start.
		let restart_path = format!(
			"{API_PREFIX}/containers/{}/restart?timeout=5",
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
	pub(in crate::engine::watch) async fn watch_exec(
		&self,
		container_name: &str,
		cmd: Vec<String>,
	) -> Result<()> {
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
