//! Test-helper `Engine` methods that exercise the watch code paths without
//! driving the full notify loop. Feature-gated so they never appear in
//! release builds.
//!
//! Extracted from `mod.rs` to keep the watcher source under the line limit;
//! the helpers all delegate to private methods on `Engine`, so they only
//! need to be reachable from the integration tests under `tests/`.

#[cfg(feature = "test-helpers")]
impl crate::engine::Engine {
	/// Test seam: copy `src` into `container` at `target` via the watch sync
	/// path, treating `src` as both the watch-rule root and the changed entry
	/// (as the initial-sync path does).
	pub async fn test_sync_to_container(
		&self,
		container: &str,
		src: &std::path::Path,
		target: &str,
	) -> crate::error::Result<()> {
		let mut ensured = std::collections::HashSet::new();
		self.sync_to_container(container, src, src, target, &mut ensured)
			.await
	}

	/// Test seam: delete the entry `path` would have written under `target`
	/// from `container`. Mirrors the live `dispatch_action` path that runs on
	/// a `Remove` notify event.
	pub async fn test_remove_from_container(
		&self,
		container: &str,
		src: &std::path::Path,
		target: &str,
	) -> crate::error::Result<()> {
		self.remove_from_container(container, src, src, target)
			.await
	}

	/// Test seam: run the watch restart action against `container_name`.
	pub async fn test_watch_restart(&self, container_name: &str) -> crate::error::Result<()> {
		self.watch_restart(container_name).await
	}

	/// Test seam: run the watch exec action (`cmd`) against `container_name`.
	pub async fn test_watch_exec(
		&self,
		container_name: &str,
		cmd: Vec<String>,
	) -> crate::error::Result<()> {
		self.watch_exec(container_name, cmd).await
	}

	/// All container names carrying this project's label (any state). Lets
	/// integration tests assert which service containers `run` did or did not
	/// create (e.g. that `--no-deps` skipped a dependency).
	pub async fn test_project_container_names(&self) -> crate::error::Result<Vec<String>> {
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
	pub async fn test_container_aliases(
		&self,
		container: &str,
	) -> crate::error::Result<Vec<String>> {
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
	pub async fn test_exec_capture(
		&self,
		container: &str,
		cmd: Vec<String>,
	) -> crate::error::Result<String> {
		use futures_util::StreamExt;

		use crate::libpod::types::exec::{ExecCreateConfig, ExecCreateResponse, ExecStartConfig};
		use crate::libpod::{LogOutput, API_PREFIX};

		let exec_cfg = ExecCreateConfig {
			cmd: Some(cmd),
			attach_stdout: Some(true),
			attach_stderr: Some(true),
			..Default::default()
		};
		let create_path = format!(
			"{API_PREFIX}/containers/{}/exec",
			crate::libpod::urlencoded(container)
		);
		let resp: ExecCreateResponse = self
			.client
			.post_json(&create_path, &exec_cfg)
			.await
			.map_err(crate::error::ComposeError::Podman)?;

		let start_cfg = ExecStartConfig {
			detach: false,
			tty: false,
		};
		let start_path = format!(
			"{API_PREFIX}/exec/{}/start",
			crate::libpod::urlencoded(&resp.id)
		);
		let start_resp = self
			.client
			.post_json_stream(&start_path, &start_cfg)
			.await
			.map_err(crate::error::ComposeError::Podman)?;
		let mut stream = crate::libpod::parse_multiplexed(start_resp.into_body());

		let mut out = String::new();
		while let Some(msg) = stream.next().await {
			if let LogOutput::StdOut { message } =
				msg.map_err(crate::error::ComposeError::Podman)?
			{
				out.push_str(&String::from_utf8_lossy(&message));
			}
		}
		Ok(out)
	}
}
