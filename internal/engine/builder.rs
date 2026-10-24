//! Constructors and builder-style setters for [`Engine`].
//!
//! Lifted out of `engine::mod` so the engine module stays under the 500-line
//! hard cap enforced by the org's `line-limit` reusable, and so the call
//! sites for "how do I make an `Engine`" stay together. The struct itself
//! and its operational methods stay in `mod.rs`.

use std::path::PathBuf;

use crate::engine::lifecycle::RunOverrides;
use crate::engine::{project::label::build_project_label_parts, Engine};
use crate::libpod::Client;

impl Engine {
	/// Create an engine for `project_name` using the working directory as the
	/// base path for relative volume mounts.
	///
	/// If the working directory cannot be resolved (the process's CWD was
	/// deleted or is unreadable at construction time), the engine falls back
	/// to an empty `base_dir` and a warning is logged. Callers that need a
	/// definite base directory (the CLI does) should use
	/// [`Engine::with_base_dir`] instead, which surfaces a missing or
	/// unreadable directory as a hard error rather than a silent empty
	/// path that later surfaces as a confusing "compose file not found".
	pub fn new(client: Client, project: String) -> Self {
		let base_dir = match std::env::current_dir() {
			Ok(dir) => dir,
			Err(e) => {
				tracing::warn!(
					"cannot resolve the current working directory ({e}); base_dir is empty \
					 and relative paths (compose files, volume mounts, env_file sources) \
					 will not resolve. Use Engine::with_base_dir to set an explicit base \
					 directory."
				);
				PathBuf::new()
			}
		};
		Self {
			client,
			project: project.clone(),
			base_dir,
			compose_files: Vec::new(),
			stop_timeout: None,
			scale_overrides: std::collections::HashMap::new(),
			pull_policy_override: None,
			no_build: false,
			quiet_pull: false,
			run_overrides: RunOverrides::default(),
			run_env_files: Vec::new(),
			run_labels: Vec::new(),
			run_no_tty: false,
			renew_anon_volumes: false,
			images_seen_present: std::sync::Mutex::new(std::collections::HashSet::new()),
			uploaded_file_digests: std::sync::Mutex::new(std::collections::HashMap::new()),
			keep_id_create: tokio::sync::Mutex::new(()),
			no_warn: false,
			pull_policy_warned: Default::default(),
			project_label: build_project_label_parts(&project),
		}
	}

	/// Create an engine with an explicit base directory, for when the compose file is not in the working directory.
	pub fn with_base_dir(client: Client, project: String, base_dir: PathBuf) -> Self {
		Self {
			client,
			project: project.clone(),
			base_dir,
			compose_files: Vec::new(),
			stop_timeout: None,
			scale_overrides: std::collections::HashMap::new(),
			pull_policy_override: None,
			no_build: false,
			quiet_pull: false,
			run_overrides: RunOverrides::default(),
			run_env_files: Vec::new(),
			run_labels: Vec::new(),
			run_no_tty: false,
			renew_anon_volumes: false,
			images_seen_present: std::sync::Mutex::new(std::collections::HashSet::new()),
			uploaded_file_digests: std::sync::Mutex::new(std::collections::HashMap::new()),
			keep_id_create: tokio::sync::Mutex::new(()),
			no_warn: false,
			pull_policy_warned: Default::default(),
			project_label: build_project_label_parts(&project),
		}
	}

	/// Set the CLI `-t/--timeout` shutdown-grace override (seconds). Builder-style.
	pub fn with_stop_timeout(mut self, timeout: Option<i32>) -> Self {
		self.stop_timeout = timeout;
		self
	}

	/// Set the CLI `--scale SERVICE=N` replica overrides. Builder-style.
	pub fn with_scale_overrides(
		mut self,
		overrides: std::collections::HashMap<String, u32>,
	) -> Self {
		self.scale_overrides = overrides;
		self
	}

	/// Set the CLI `up` image-acquisition overrides: `--pull <policy>`,
	/// `--no-build`, and `--quiet-pull`. Builder-style.
	pub fn with_up_overrides(
		mut self,
		pull_policy: Option<String>,
		no_build: bool,
		quiet_pull: bool,
	) -> Self {
		self.pull_policy_override = pull_policy;
		self.no_build = no_build;
		self.quiet_pull = quiet_pull;
		self
	}

	/// Record the compose files this engine was built from, so containers it
	/// creates carry them as a `podup.config-files` label and `ls` can report
	/// where a project's compose file lives. Builder-style; additive, so an
	/// embedder that does not call it simply gets no label.
	pub fn with_compose_files(mut self, files: Vec<PathBuf>) -> Self {
		self.compose_files = files;
		self
	}

	/// Set the CLI `run`-only flag overrides (`-u/-w/--entrypoint/-v/-p/-i/
	/// --no-deps`). Builder-style; consumed by [`Engine::run`].
	pub fn with_run_overrides(mut self, overrides: RunOverrides) -> Self {
		self.run_overrides = overrides;
		self
	}

	/// Set the global `--env-file` paths that also seed a one-off `run`
	/// container's environment (`docker compose run --env-file`: env-file <
	/// service `environment:` < `-e`). Builder-style; consumed by
	/// [`Engine::run`]. Resolved relative to the engine's base dir.
	pub fn with_run_env_files(mut self, env_files: Vec<String>) -> Self {
		self.run_env_files = env_files;
		self
	}

	/// Set the CLI `docker compose run -l/--label KEY=VAL` ad-hoc labels for the
	/// one-off `run` container. Builder-style; consumed by [`Engine::run`].
	pub fn with_run_labels(mut self, labels: Vec<String>) -> Self {
		self.run_labels = labels;
		self
	}

	/// Set the CLI `run -T/--no-TTY` flag. Builder-style.
	///
	/// Carried here rather than on [`RunOverrides`] for the reason that struct
	/// already documents: it is public and not `#[non_exhaustive]`, so a new
	/// field is a breaking change. `run_env_files` and `run_labels` are on the
	/// engine for exactly this; cargo-semver-checks flagged this field when it
	/// was placed on `RunOverrides`.
	pub fn with_run_no_tty(mut self, no_tty: bool) -> Self {
		self.run_no_tty = no_tty;
		self
	}

	/// Set the CLI `up -V/--renew-anon-volumes` flag. Builder-style; when set,
	/// recreating a container also removes its old anonymous volumes.
	pub fn with_renew_anon_volumes(mut self, renew: bool) -> Self {
		self.renew_anon_volumes = renew;
		self
	}

	/// Set the CLI `--no-warn` flag. Builder-style; when set, the engine
	/// suppresses the host-binding / privilege-escalation warnings it emits
	/// during `up`/`create`/`run`/`exec`. Operators who deliberately wrote
	/// `privileged: true` (or any other host-binding mode) into the compose
	/// file use this to silence the per-run warning; `config`'s surface-mode
	/// listing is unaffected (`config` is the "show me what will happen"
	/// command, where the warning is the whole point).
	pub fn with_no_warn(mut self, no_warn: bool) -> Self {
		self.no_warn = no_warn;
		self
	}
}
