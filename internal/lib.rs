//! The library target behind the `podup` command: compose parsing, variable
//! substitution, topological ordering, and the async engine that drives
//! container lifecycle through Podman's native libpod REST API over a Unix
//! socket or Windows named pipe.
//!
//! It is internal code, not a product. podup is consumed as a program: other
//! products install it and run the `podup` binary, and nothing outside this
//! repository links this crate. Its items can change in any release; the
//! supported interface is the command line (commands, flags, exit codes and
//! `--format json` output).

// `unsafe` is denied crate-wide; the few modules that need libc FFI opt back in
// locally with `#![allow(unsafe_code)]` and a soundness comment per block, so a
// new `unsafe` block elsewhere fails the build.
#![deny(unsafe_code)]
// Every public item carries a doc comment (the organisation's style standard),
// and denying missing docs keeps that true without anyone having to notice it
// a year later. Turned on at 35 outstanding items; it is cheap to adopt at that
// size and expensive at three hundred.
#![deny(missing_docs)]

/// `podup autostart`: render and manage a rootless `systemctl --user` unit that
/// brings a compose stack up at boot (service mode).
pub mod autostart;
/// Compose-file parsing, `extends:`/`include:` resolution, and topological
/// service ordering.
pub mod compose;
pub(crate) mod dotenv;
pub(crate) mod engine;
/// `env_file:` loading: KEY=VALUE pairs from a service's declared files.
pub mod env_file;
pub(crate) mod error;
pub(crate) mod filesystem;
pub(crate) mod libpod;
/// Podman socket connection helpers.
pub mod podman;
/// Port-mapping parser for the docker-compose `ports:` format variants.
pub mod ports;
/// Quadlet export: translate a parsed compose file into Podman systemd units.
pub mod quadlet;
/// Memory and CPU value parsers shared by the engine and tests.
pub mod size;
/// Docker Compose `${VAR}`/`$VAR` substitution over raw YAML before parsing.
pub mod substitute;
pub(crate) mod timestamp;
/// Terminal colour/styling, honouring `--ansi`, `NO_COLOR`, and TTY detection.
pub mod ui;
pub(crate) mod units;
/// Secure self-update for the `podup` binary (signature-verified release fetch).
#[cfg(feature = "update")]
pub mod update;

/// Compose entry points: the parser variants and service-ordering helpers,
/// re-exported at the crate root for callers.
pub use compose::{
	parse_file_with_env_files, parse_files_with_env_files, parse_files_with_env_files_interp,
	parse_str, parse_str_raw, resolve_levels, resolve_order, validate_config,
};
/// The lifecycle `Engine` and its per-command option/override types, plus the
/// project-name/listing helpers: the surface a CLI drives compose operations
/// through.
pub use engine::{
	is_safe_project_name, list_projects, list_projects_filtered, resolve_image_digests,
	resolve_network_name, retain_active_profiles, retain_active_profiles_with_targets,
	surface_host_modes, validate_stop_timeout, AttachOptions, AttachOutcome, AttachSummary,
	BuildOptions, CommitOptions, CpOptions, Engine, EventsOptions, ExecOptions, ImagesOptions,
	LogsDisplay, LogsOptions, LsOptions, ProjectLock, PsDisplayOptions, PsFilterOptions, PsOptions,
	PullOptions, PushOptions, RunOptions, RunOverrides, StatsOptions, VolumesDisplayOptions,
	VolumesOptions, DEFAULT_LOG_TAIL,
};
/// Return the runtime value of `security_opt`'s `no-new-privileges` key:
/// `Some(true)` when the engine will apply it, `Some(false)` when an entry
/// explicitly disables it, `None` when no entry matched. Surfaced for the
/// audit module, which must read the resolved value rather than the
/// compose-side text (`#1743`).
pub fn effective_no_new_privileges(service: &crate::compose::types::Service) -> Option<bool> {
	crate::engine::container::parse_security_opts(service).no_new_privileges
}
/// Ports the parse-time port-exposure warning would flag: every entry
/// the engine sees as published without an explicit host IP, so the
/// bind falls on every interface. Each tuple is `(service_name,
/// host_port_label)`; `host_port_label` is the operator-visible port
/// (a single number for `8080:80`, a range string for `published:
/// "8080-8090"`). Surfaced for the audit module so its
/// `port_published_on_all_interfaces` check is the same notion
/// `up`/`config` use rather than a divergent second opinion (`#1835`).
pub fn ports_published_on_all_interfaces(
	file: &crate::compose::types::ComposeFile,
) -> Vec<(String, String)> {
	crate::compose::diagnostics::ports_published_on_all_interfaces(file)
		.into_iter()
		.map(|e| (e.service, e.host))
		.collect()
}
/// Ports published with an explicit wildcard host IP (`0.0.0.0` or
/// `::`). Same tuple shape as
/// [`ports_published_on_all_interfaces`] and the same per-mapping
/// counting, but the inverse predicate: this one fires on the IP-typed
/// wildcard, not on "no IP given", so a compose file can fire both
/// checks on different mappings but never on the same one (`#1881`).
pub fn ports_published_on_wildcard(
	file: &crate::compose::types::ComposeFile,
) -> Vec<(String, String)> {
	crate::compose::diagnostics::ports_published_on_wildcard(file)
		.into_iter()
		.map(|e| (e.service, e.host))
		.collect()
}
/// The memory cap the engine will forward into `LinuxMemory.limit`, in
/// bytes. Top-level `mem_limit:` wins; the modern
/// `deploy.resources.limits.memory:` block only fills in a value the
/// top level left unset. An unparseable value is `None`; the literal
/// `"-1"` is forwarded verbatim as Podman's "no cap" sentinel.
///
/// Exists for the podup binary's `audit` command, so its
/// `no_memory_limit` and `swap_unbounded` checks read the same value
/// the engine will build. Not part of the library's supported public
/// surface (`#1894`).
#[doc(hidden)]
pub fn effective_memory_limit(service: &crate::compose::types::Service) -> Option<i64> {
	crate::engine::effective_memory_limit(service)
}
/// The CFS CPU quota the engine will forward into `LinuxCPU.quota`, in
/// microseconds over `cpu_period` (default 100_000). `cpu_quota:` wins
/// when present, including `-1` (Docker's "unlimited" sentinel) and
/// `0`. Otherwise derived from `cpus:` (top-level first, then
/// `deploy.resources.limits.cpus:`) divided by 10_000 to convert
/// nano-CPUs to an OCI quota over the default 100ms period.
///
/// Exists for the podup binary's `audit` command, so its
/// `no_cpu_limit` check reads the same value the engine forwards into
/// the OCI spec. Not part of the library's supported public surface
/// (`#1894`).
#[doc(hidden)]
pub fn effective_cpu_quota(service: &crate::compose::types::Service) -> Option<i64> {
	crate::engine::effective_cpu_quota(service)
}
/// The crate's error type and `Result` alias, surfaced so callers handle one
/// error enum across parsing and engine calls.
pub use error::{ComposeError, Result};
/// The libpod `Client`, surfaced for callers that talk to Podman directly.
pub use libpod::Client;
/// The body type the streaming libpod methods return.
///
/// The streaming `get_stream`, `post_json_stream`, `post_empty_stream`,
/// `post_bytes_stream`, `post_stream_body`, and `post_json_stream_within`
/// methods hand back a `Response<DrivenBody>`. The wrapper drives the
/// underlying HTTP/1 connection in line with `Body::poll_frame`, so frames
/// the connection has just decoded become visible to the body poll
/// without a cross-task wake-up. Dropping the body closes the connection:
/// the streaming socket lives inside the body, not the [`Client`], so a
/// caller that abandons the stream still frees the daemon-side resource
/// (#1900).
pub use libpod::DrivenBody;
/// The libpod error type carried inside [`ComposeError::Podman`], with the
/// predicates the engine's own retry paths use.
///
/// An embedding daemon has to tell a transport fault it should retry from a
/// rejection it should not, and the only alternative to these predicates is
/// matching on the message text, which breaks silently the day libpod rewords
/// one.
pub use libpod::PodmanError;
/// Log frames as libpod delivers them, and the stream parsers that produce
/// them, for callers routing container output somewhere other than this
/// process's stdout.
pub use libpod::{parse_json_lines, parse_multiplexed, parse_raw, LogOutput};

/// Internal parsers exposed only under `test-helpers` for fuzzing and tests.
///
/// These are not part of the public API (the feature is off by default, so the
/// published crate does not expose them); they let the fuzz harness reach the
/// crate-private dotenv parser, the libpod stream framer, and the
/// container→host tar extractor.
#[cfg(feature = "test-helpers")]
pub mod fuzz_api {
	pub use crate::dotenv::parse as dotenv_parse;
	pub use crate::engine::copy::archive::extract_tar_guarded;
	pub use crate::libpod::types::stream::{
		parse_frame, record_stream_bytes, take_json_line, MAX_STREAM_BUF,
	};
	pub use crate::libpod::PodmanError;
}
