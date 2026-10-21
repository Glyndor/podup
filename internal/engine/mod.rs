//! Container orchestration engine.
//!
//! Translates a parsed [`ComposeFile`](crate::compose::types::ComposeFile) into Podman API calls via the libpod REST API.

pub(crate) mod build;
mod builder;
pub(crate) mod container;
pub(crate) mod context;
/// `pub(crate)` so the fuzz harness behind the `test-helpers` feature can
/// reach `archive::extract_tar_guarded` without widening the published API:
/// the chain stays `engine → copy → archive` all `pub(crate)` (or stricter),
/// which a third-party crate cannot see.
pub(crate) mod copy;
pub(crate) mod events;
pub(crate) mod project;
pub(crate) mod terminal;
pub use events::EventsOptions;
mod image;
pub use build::{BuildOptions, PullOptions, PushOptions};
pub use copy::CpOptions;
pub use image::{resolve_image_digests, CommitOptions};
pub use lifecycle::{validate_stop_timeout, RunOptions, RunOverrides};
pub use project::ProjectLock;
mod lifecycle_hooks;

mod container_config;
pub(crate) use container_config::build_log_config;
pub(crate) use container_config::resources::{effective_cpu_quota, effective_memory_limit};
#[cfg(test)]
mod fake_podman;
mod health;
mod lifecycle;
mod network;
/// Re-exported so `startup::config_render` (in the binary crate) can call the
/// same function `up` uses, instead of duplicating the resolution rule.
pub use network::resolve_network_name;
mod pod;
pub(crate) use pod::pod_network_mode;
pub use project::{list_projects, list_projects_filtered, LsOptions};
pub use project::{retain_active_profiles, retain_active_profiles_with_targets};
pub(crate) mod query;
pub use query::{
	AttachOptions, AttachOutcome, AttachSummary, ExecOptions, ImagesOptions, LogsDisplay,
	LogsOptions, PsDisplayOptions, PsFilterOptions, PsOptions, DEFAULT_LOG_TAIL,
};
mod secrets;
mod stats;
pub use project::is_safe_project_name;
pub use stats::StatsOptions;
mod volume;
pub use volume::{VolumesDisplayOptions, VolumesOptions};
mod volume_mounts;
#[cfg(feature = "watch")]
mod watch;

use std::path::PathBuf;

use crate::compose::types::Service;
use crate::error::{ComposeError, Result};
use crate::libpod::Client;

use crate::engine::project::label::ProjectLabelParts;

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

/// Handle through which all Podman operations for a project are dispatched.
pub struct Engine {
	pub(super) client: Client,
	pub(super) project: String,
	pub(super) base_dir: PathBuf,
	/// Absolute compose-file paths this engine was built from, in `-f` order.
	/// Stamped onto every container as `podup.config-files` so `ls` can report
	/// them: projects are discovered by label, with no other record of where
	/// their compose file lives. Empty when the caller did not supply them, in
	/// which case the label is omitted rather than written blank.
	pub(super) compose_files: Vec<PathBuf>,
	/// Optional CLI `-t/--timeout` override (seconds) for container shutdown
	/// grace; when set it takes precedence over each service's
	/// `stop_grace_period`. `None` falls back to the per-service value.
	pub(super) stop_timeout: Option<i32>,
	/// CLI `--scale SERVICE=N` overrides (from `up --scale` and the `scale`
	/// subcommand); when a service is present it takes precedence over the
	/// compose `scale:`/`deploy.replicas` value. Empty falls back to compose.
	pub(super) scale_overrides: std::collections::HashMap<String, u32>,
	/// CLI `up --pull <policy>` override; takes precedence over each service's
	/// `pull_policy`. `None` falls back to the per-service value.
	pub(super) pull_policy_override: Option<String>,
	/// CLI `up --no-build`: never build images, even for services with a
	/// `build:` section (they fall back to pulling/using an existing image).
	pub(super) no_build: bool,
	/// CLI `up --quiet-pull`: suppress image-pull progress output.
	pub(super) quiet_pull: bool,
	/// CLI `run`-only flag overrides (user/workdir/entrypoint/volume/publish/
	/// interactive/no-deps); empty by default.
	pub(super) run_overrides: lifecycle::RunOverrides,
	/// Global `--env-file` paths that double as `docker compose run --env-file`:
	/// their contents seed a one-off `run` container's environment at the lowest
	/// precedence (env-file < service `environment:` < `-e`). Resolved relative
	/// to `base_dir`; empty by default. Kept off the frozen public
	/// [`lifecycle::RunOverrides`] struct so the library API stays stable.
	pub(super) run_env_files: Vec<String>,
	/// CLI `docker compose run -l/--label KEY=VAL` ad-hoc labels for the one-off
	/// `run` container; empty by default. Kept off the frozen public
	/// [`lifecycle::RunOverrides`] struct so the library API stays stable.
	pub(super) run_labels: Vec<String>,
	/// CLI `run -T/--no-TTY`: opt out of the pseudo-TTY.
	pub(super) run_no_tty: bool,
	/// CLI `up -V/--renew-anon-volumes`: when recreating a container, also remove
	/// its old anonymous volumes instead of leaving them orphaned.
	pub(super) renew_anon_volumes: bool,
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
	pub(super) images_seen_present: std::sync::Mutex<std::collections::HashSet<String>>,
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
	pub(super) keep_id_create: tokio::sync::Mutex<()>,
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
	pub(super) uploaded_file_digests: std::sync::Mutex<std::collections::HashMap<String, [u8; 32]>>,
	/// CLI `--no-warn`: suppress the host-binding / privilege-escalation
	/// warnings the engine emits during `up`/`create`/`run`/`exec`. The
	/// operator wrote the compose file deliberately, so the default-warning
	/// behaviour is opt-out per run rather than per command. `config`'s
	/// surface-mode listing is unaffected; `config` is the "show me what will
	/// happen" command, where the warnings stay visible there.
	pub(super) no_warn: bool,
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
// JSON serialisation helpers
// ---------------------------------------------------------------------------

/// Serialise `v` to a compact JSON string for embedding into a libpod query
/// parameter or NDJSON row.
///
/// Six sites flow through this: five in [`build`](super::build) (`cachefrom`,
/// `buildargs`, `labels`, `secrets`, `cacheto`) and one in
/// [`events`](super::events) (the `--format json` event row). Each of them
/// used to call `serde_json::to_string(...).unwrap_or_default()`, which
/// silently emitted `""` on a serialisation failure; the empty string was
/// then placed verbatim into the query parameter, where libpod treats it as
/// "no value provided", so the user's args were ignored and the build ran
/// with image defaults. The events side dropped the row and corrupted the
/// NDJSON stream a parser was reading line-by-line (#1366).
///
/// `what` names the offending field in the error so the operator sees which
/// value was rejected.
pub(super) fn to_query_json<T: serde::Serialize>(what: &str, v: &T) -> Result<String> {
	serde_json::to_string(v).map_err(|e| ComposeError::Build(format!("invalid {what}: {e}")))
}

/// Serialise `v` to a pretty-printed JSON string for emitting as the final
/// `--format json` output of a list command.
///
/// Five sites flow through this: `ls` ([`projects::list_projects_filtered`]),
/// `images` ([`Engine::images_with_services`]), `top`
/// ([`Engine::top_with_options`]), `ps` ([`Engine::ps_filtered_with_display`]),
/// and `volumes` ([`Engine::list_volumes_with_display`]). Each used to call
/// `serde_json::to_string_pretty(...).unwrap_or_default()`, which silently
/// emitted an empty string on a serialisation failure; the command then
/// printed the empty string and exited 0, so a script consuming
/// `podup <cmd> --format json` received an empty document indistinguishable
/// from "no results" (#1444). Unlike the NDJSON path in
/// [`to_query_json`](self)/[`events`](super::events), where one row can be
/// dropped and the stream continue, `--format json` is the *whole* output,
/// so a failure must propagate as an error and exit non-zero.
///
/// `what` names the offending field in the error so the operator sees which
/// row or document type was rejected.
pub(super) fn to_pretty_json<T: serde::Serialize>(what: &str, v: &T) -> Result<String> {
	serde_json::to_string_pretty(v).map_err(|e| ComposeError::Build(format!("invalid {what}: {e}")))
}

/// Write one log frame without creating a lossy `Cow` for valid UTF-8.
///
/// `String::from_utf8_lossy` allocates a `Cow<String>` even on the happy path;
/// `std::str::from_utf8` returns `&str` directly with no allocation. The
/// lossy path is the rare one (an arbitrary byte stream from `run`/`exec`/
/// hook output), so a single shared helper keeps the two call sites
/// (`Engine::run` and `run_lifecycle_hook`) honest and matches the previous
/// observable output (#1369).
pub(crate) fn write_frame<W: std::io::Write>(out: &mut W, bytes: &[u8]) -> std::io::Result<()> {
	match std::str::from_utf8(bytes) {
		Ok(text) => out.write_all(text.as_bytes()),
		Err(_) => out.write_all(String::from_utf8_lossy(bytes).as_bytes()),
	}
}

/// Emit one `tracing::warn!` per active host-binding / privilege-escalation mode
/// across every service in `file`.
///
/// The `config` command uses this to surface the active modes at the default
/// log level (CI logs see them even when the operator never runs `up`). It is
/// deliberately not gated on `--no-warn`; `config` is the "show me what will
/// happen" command, where the warning is the whole point. The live
/// `up`/`create`/`run`/`exec` paths emit the same warnings per-call but honour
/// `--no-warn`.
pub fn surface_host_modes(file: &crate::compose::types::ComposeFile) {
	for (name, service) in &file.services {
		for w in self::container::check_host_mode(name, service) {
			tracing::warn!("{}", w.message);
		}
	}
}

// ---------------------------------------------------------------------------
// Replica resolution helpers (see `project/replicas.rs`)
// ---------------------------------------------------------------------------

/// How many replicas a service declares in the file, before any `--scale` on
/// the current invocation. Split out of [`Engine::resolve_replicas`] so a
/// caller with no `Engine` (autostart's start mode) reads the same rule rather
/// than restating it.
pub(crate) fn declared_replicas(service: &Service) -> usize {
	service
		.scale
		.or(service.deploy.as_ref().and_then(|d| d.replicas))
		.unwrap_or(1) as usize
}

/// The container name a service resolves to at exactly one replica.
///
/// This is [`Engine::replica_names_for`] at `count == 1`, extracted so
/// autostart's start mode names the container the engine actually created
/// instead of spelling the rule out a second time. The two are pinned together
/// by `naming_agrees_with_the_engine_at_one_replica`; without that test this
/// would be a copy that drifts silently, and the unit would name a container
/// that does not exist.
pub(crate) fn sole_replica_name(project: &str, service_name: &str, service: &Service) -> String {
	match &service.container_name {
		Some(explicit) => explicit.clone(),
		None => format!("{project}-{service_name}-1"),
	}
}

mod start_mode;

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "to_query_json_tests.rs"]
mod to_query_json_tests;

#[cfg(test)]
#[path = "to_pretty_json_tests.rs"]
mod to_pretty_json_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod stream_end_tests;
#[cfg(unix)]
#[cfg(test)]
mod wait_timeout_tests;
