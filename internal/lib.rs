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

// One folder per concern. Each module the rest of the crate (or a consumer of
// the library) reaches by name is re-exported from here under that name, so
// moving a module between folders never changes a path.
mod backends;
mod connection;
mod errors;
mod parsing;
/// Terminal colour/styling, honouring `--ansi`, `NO_COLOR`, and TTY detection.
pub mod ui;
/// Secure self-update for the `podup` binary (signature-verified release fetch).
#[cfg(feature = "update")]
pub mod update;

pub use backends::autostart;
pub(crate) use backends::engine;
pub use backends::quadlet;
pub(crate) use connection::libpod;
pub use connection::podman;
pub use parsing::compose;
use parsing::input;
pub use parsing::substitute;
use parsing::values;

pub(crate) use errors::error;
#[doc(hidden)]
pub use errors::service_hint;
pub use input::env_file;
pub(crate) use input::{dotenv, filesystem};
pub use values::{ports, size};
pub(crate) use values::{timestamp, units};

/// Compose entry points: the parser variants and service-ordering helpers,
/// re-exported at the crate root for callers.
pub use compose::{
	parse_file_with_env_files, parse_files_with_env_files, parse_files_with_env_files_interp,
	parse_files_with_env_files_interp_in, parse_str, parse_str_raw, resolve_levels, resolve_order,
	validate_config, warn_unscheduled_jobs,
};
/// The lifecycle `Engine` and its per-command option/override types, plus the
/// project-name/listing helpers: the surface a CLI drives compose operations
/// through.
pub use engine::{
	is_safe_project_name, list_projects, list_projects_filtered, resolve_image_digests,
	resolve_network_name, retain_active_profiles, retain_active_profiles_with_targets,
	retain_up_scope, surface_host_modes, validate_stop_timeout, AttachOptions, AttachOutcome,
	AttachSummary, BuildOptions, CommitOptions, ContainerSnapshot, CpOptions, Engine,
	EventsOptions, ExecOptions, ImagesOptions, LogsDisplay, LogsOptions, LsOptions, ProjectLock,
	PsDisplayOptions, PsFilterOptions, PsOptions, PullOptions, PushOptions, RunOptions,
	RunOverrides, StatsOptions, VolumesDisplayOptions, VolumesOptions, DEFAULT_LOG_TAIL,
};
/// The image reference a service runs: its `image:`, or for a build-only service
/// the tag the build step gives it (`build.tags[0]`, else `{project}-{service}:latest`).
/// `config --images` prints this, and it is the same rule `up` and `build` apply, so
/// the listing names the image that will exist. `None` for a service with neither.
pub fn service_image_name(
	project: &str,
	service_name: &str,
	service: &compose::types::Service,
) -> Option<String> {
	match (&service.image, &service.build) {
		(Some(image), _) => Some(image.clone()),
		(None, Some(build)) => Some(engine::build::primary_build_tag(
			project,
			service_name,
			None,
			build.tags(),
		)),
		(None, None) => None,
	}
}
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
/// The same predicate as [`ports_published_on_all_interfaces`], scoped to one
/// container instead of to the whole file. The audit walks containers that
/// are not services of the file: an init step publishes ports of its own,
/// and its name is the one the audit reports it under.
pub fn ports_published_on_all_interfaces_in(
	service_name: &str,
	service: &crate::compose::types::Service,
) -> Vec<(String, String)> {
	crate::compose::diagnostics::ports_published_on_all_interfaces_in(service_name, service)
		.into_iter()
		.map(|e| (e.service, e.host))
		.collect()
}
/// The same predicate as [`ports_published_on_wildcard`], scoped to one
/// container instead of to the whole file. Companion of
/// [`ports_published_on_all_interfaces_in`].
pub fn ports_published_on_wildcard_in(
	service_name: &str,
	service: &crate::compose::types::Service,
) -> Vec<(String, String)> {
	crate::compose::diagnostics::ports_published_on_wildcard_in(service_name, service)
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
/// The PID cap the engine forwards into `LinuxPids.limit`. Top-level
/// `pids_limit` wins over deploy, including the unlimited sentinel `-1`.
/// Shared with the binary's audit; not a supported library interface.
#[doc(hidden)]
pub fn effective_pids_limit(service: &crate::compose::types::Service) -> Option<i64> {
	crate::engine::effective_pids_limit(service)
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
	pub use crate::dotenv::{parse as dotenv_parse, parse_strict as dotenv_parse_strict};
	pub use crate::engine::copy::archive::extract_tar_guarded;
	pub use crate::libpod::types::stream::{
		parse_frame, record_stream_bytes, take_json_line, MAX_STREAM_BUF,
	};
	pub use crate::libpod::PodmanError;

	/// The `.dockerignore` matcher.
	pub use crate::engine::context::fuzz_hooks::{
		evaluate, glob_match, is_ignored, pattern_matches,
	};
	/// The whole-second interval syntax of the pull policies.
	pub fn parse_interval(value: &str) -> Option<u32> {
		crate::engine::build::fuzz_parse_interval(value)
	}
	/// The daemon's JSON response types.
	pub use crate::libpod::types::fuzz_hook::decode as libpod_json_decode;
	/// The cron to `OnCalendar=` conversion.
	pub use crate::quadlet::fuzz_hook::{convert as cron_convert, Converted as CronConverted};
	/// The self-update parsers: the version, the digest and the release tag.
	#[cfg(feature = "update")]
	pub use crate::update::fuzz_hook::{expected_digest, parse_latest_tag, parse_version, Version};

	/// `sanitize_name`, as an owned string plus whether it returned the input
	/// untouched (`true`) or had to escape something (`false`).
	pub fn sanitize_name(s: &str) -> (String, bool) {
		let out = crate::error::sanitize_name(s);
		let borrowed = matches!(out, std::borrow::Cow::Borrowed(_));
		(out.into_owned(), borrowed)
	}
}
