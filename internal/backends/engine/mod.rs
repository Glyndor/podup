//! Container orchestration engine.
//!
//! Translates a parsed [`ComposeFile`](crate::compose::types::ComposeFile) into Podman API calls via the libpod REST API.

// One folder per concern. Everything the rest of the crate reaches as
// `crate::engine::<module>` is re-exported from here, so moving a module
// between folders never changes a path outside the engine.
mod builder;
#[cfg(test)]
mod fake_podman;
mod host_files;
mod images;
mod lifecycle;
mod objects;
mod observe;
pub(crate) mod project;
mod workload;

pub(crate) use host_files::copy;
#[cfg(feature = "watch")]
use host_files::watch;
pub(crate) use images::build;
pub(crate) use images::context;
use images::image;
use objects::network;
use objects::secrets;
use objects::volume;
use objects::volume_mounts;
pub(crate) use observe::events;
pub(crate) use observe::query;
use observe::stats;
pub(crate) use observe::terminal;
pub(crate) use workload::container;
use workload::container_config;
use workload::declared;
use workload::pod;

pub use build::{BuildOptions, PullOptions, PushOptions};
pub use copy::CpOptions;
pub use events::EventsOptions;
pub use image::{resolve_image_digests, CommitOptions};
pub use lifecycle::{validate_stop_timeout, ContainerSnapshot, RunOptions, RunOverrides};
pub use project::ProjectLock;

pub(crate) use container_config::build_log_config;
pub(crate) use container_config::resources::{
	effective_cpu_quota, effective_memory_limit, effective_pids_limit, is_known_ulimit,
};
pub(crate) use network::apply_default_isolation;
/// Re-exported so `startup::config_render` (in the binary crate) can call the
/// same function `up` uses, instead of duplicating the resolution rule.
pub use network::resolve_network_name;
pub(crate) use pod::pod_network_mode;
pub use project::is_safe_project_name;
pub use project::{list_projects, list_projects_filtered, LsOptions};
pub use project::{retain_active_profiles, retain_active_profiles_with_targets, retain_up_scope};
pub use query::{
	AttachOptions, AttachOutcome, AttachSummary, ExecOptions, ImagesOptions, LogsDisplay,
	LogsOptions, PsDisplayOptions, PsFilterOptions, PsOptions, DEFAULT_LOG_TAIL,
};
pub use stats::StatsOptions;
pub use volume::{VolumesDisplayOptions, VolumesOptions};

use std::path::PathBuf;

/// The no-watch `Engine::watch` stub below is the only reader; the import is
/// gated with it so a `watch` build does not carry an unused one.
#[cfg(not(feature = "watch"))]
use crate::error::Result;
use crate::libpod::Client;

use crate::engine::project::label::ProjectLabelParts;

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

/// Handle through which all Podman operations for a project are dispatched.
pub struct Engine {
	pub(crate) client: Client,
	pub(crate) project: String,
	pub(crate) base_dir: PathBuf,
	/// Absolute compose-file paths this engine was built from, in `-f` order.
	/// Stamped onto every container as `podup.config-files` so `ls` can report
	/// them: projects are discovered by label, with no other record of where
	/// their compose file lives. Empty when the caller did not supply them, in
	/// which case the label is omitted rather than written blank.
	pub(crate) compose_files: Vec<PathBuf>,
	/// Optional CLI `-t/--timeout` override (seconds) for container shutdown
	/// grace; when set it takes precedence over each service's
	/// `stop_grace_period`. `None` falls back to the per-service value.
	pub(crate) stop_timeout: Option<i32>,
	/// CLI `--scale SERVICE=N` overrides (from `up --scale` and the `scale`
	/// subcommand); when a service is present it takes precedence over the
	/// compose `scale:`/`deploy.replicas` value. Empty falls back to compose.
	pub(crate) scale_overrides: std::collections::HashMap<String, u32>,
	/// CLI `up --pull <policy>` override; takes precedence over each service's
	/// `pull_policy`. `None` falls back to the per-service value.
	pub(crate) pull_policy_override: Option<String>,
	/// CLI `up --no-build`: never build images, even for services with a
	/// `build:` section (they fall back to pulling/using an existing image).
	pub(crate) no_build: bool,
	/// CLI `up --quiet-pull`: suppress image-pull progress output.
	pub(crate) quiet_pull: bool,
	/// CLI `run`-only flag overrides (user/workdir/entrypoint/volume/publish/
	/// interactive/no-deps); empty by default.
	pub(crate) run_overrides: lifecycle::RunOverrides,
	/// Global `--env-file` paths that double as `docker compose run --env-file`:
	/// their contents seed a one-off `run` container's environment at the lowest
	/// precedence (env-file < service `environment:` < `-e`). Resolved relative
	/// to `base_dir`; empty by default. Kept off the frozen public
	/// [`lifecycle::RunOverrides`] struct so the library API stays stable.
	pub(crate) run_env_files: Vec<String>,
	/// CLI `docker compose run -l/--label KEY=VAL` ad-hoc labels for the one-off
	/// `run` container; empty by default. Kept off the frozen public
	/// [`lifecycle::RunOverrides`] struct so the library API stays stable.
	pub(crate) run_labels: Vec<String>,
	/// CLI `run -T/--no-TTY`: opt out of the pseudo-TTY.
	pub(crate) run_no_tty: bool,
	/// Where an attached session reads the caller's standard input from.
	///
	/// The process's own stdin, unless an embedder (or a test) supplies its
	/// own. A session that forwards its stdin cannot be tested against the
	/// terminal attached to the test runner: the read would block, or steal
	/// the developer's keystrokes.
	pub(crate) input: crate::engine::query::InputSource,
	/// CLI `up -V/--renew-anon-volumes`: when recreating a container, also remove
	/// its old anonymous volumes instead of leaving them orphaned.
	pub(crate) renew_anon_volumes: bool,
	/// Image references this engine observed present on the host during the
	/// current invocation, recorded by the prefetch stage so the per-service
	/// pull site does not ask again.
	///
	/// Only ever a record of what this engine saw itself, this run: it is not
	/// persisted and not shared between engines. Two engines against different
	/// sockets must not pool observations: a process-wide cache would let one
	/// skip a pull for an image that only exists on the other's host, which
	/// matters wherever one process holds more than one engine (the
	/// integration suite does).
	pub(crate) images_seen_present: std::sync::Mutex<std::collections::HashSet<String>>,
	/// Held across the `containers/create` request of every container whose
	/// user namespace is `keep-id` (bare or with options), so this engine
	/// issues those creates one at a time (#1955).
	///
	/// Podman's API service computes the `keep-id` mapping racily when several
	/// such creates arrive at once: measured on 5.7.0, one container now and
	/// then comes back mapped as `1000:0:1` alone, and crun then cannot write
	/// the default `net.ipv4.ping_group_range=0 0` sysctl because container
	/// GID 0 is unmapped, so the container is left in `created`. Creating in
	/// series never failed; starting in series did not help, so the start
	/// stays outside the lock. Reported upstream as
	/// podman-container-tools/podman#29848. Two podup processes creating
	/// `keep-id` containers at the same moment can still race.
	pub(crate) keep_id_create: tokio::sync::Mutex<()>,
	/// SHA-256 of every `file:` secret/config this engine actually uploaded
	/// during the current invocation, keyed by the project-scoped Podman
	/// secret name `create_project_secrets` used to create it. Populated
	/// before the per-service label build so `config_hash` can fold the
	/// bytes it shipped into the label instead of reading the host file
	/// again: a re-read can land on bytes that changed between the upload
	/// and the label build (image acquisition, a `depends_on` wait), and
	/// the container would then carry the label of bytes it never
	/// mounted. Empty when no `file:` source was uploaded yet (e.g.
	/// `config --hash`, autostart's start mode) — `config_hash` falls back
	/// to its file-read behaviour in that case, which is exactly what those
	/// callers always saw.
	pub(crate) uploaded_file_digests: std::sync::Mutex<std::collections::HashMap<String, [u8; 32]>>,
	/// CLI `--no-warn`: suppress the host-binding / privilege-escalation
	/// warnings the engine emits during `up`/`create`/`run`/`exec`. The
	/// operator wrote the compose file deliberately, so the default-warning
	/// behaviour is opt-out per run rather than per command. `config`'s
	/// surface-mode listing is unaffected; `config` is the "show me what will
	/// happen" command, where the warnings stay visible there.
	pub(crate) no_warn: bool,
	/// Services already warned about missing last-pull tracking on this engine.
	pull_policy_warned: std::sync::Mutex<std::collections::HashSet<String>>,
	/// The pre-URL-encoded libpod `filters` JSON object that scopes a
	/// container-list call to this project's `podup.project={name}` label,
	/// plus the raw `podup.project={name}` label string. Built once per
	/// [`Engine`] so call sites do not pay `format!` + `serde_json::to_string`
	/// + `urlencoded` on every invocation (#1364).
	project_label: ProjectLabelParts,
}

impl Engine {
	/// Watch for file changes and apply the service's `develop.watch` rules. Returns an error when the `watch` feature is disabled.
	#[cfg(not(feature = "watch"))]
	pub async fn watch(&self, _file: &crate::compose::types::ComposeFile) -> Result<()> {
		Err(crate::error::ComposeError::Unsupported(
			"watch requires the 'watch' feature".into(),
		))
	}
}

// ---------------------------------------------------------------------------
// Free-function helpers, one module per concern
// ---------------------------------------------------------------------------

mod encoding;
pub use declared::host_modes::surface_host_modes;
pub(crate) use declared::replicas::{declared_replicas, sole_replica_name};
pub(crate) use encoding::json::{to_pretty_json, to_query_json};
pub(crate) use encoding::log_frame::write_frame;

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;

#[cfg(test)]
mod stream_end_tests;
#[cfg(unix)]
#[cfg(test)]
mod wait_timeout_tests;

#[cfg(all(test, unix))]
mod ownership_tests;
