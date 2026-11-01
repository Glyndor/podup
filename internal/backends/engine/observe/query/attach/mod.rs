//! Attached up: public options, stream monitoring and abort cleanup.

mod abort;
mod phase;
mod streams;

use crate::compose::types::ComposeFile;
use crate::error::Result;

use super::Engine;

/// How an attached `up` stopped streaming.
///
/// The distinction has to survive back to the caller because the four endings
/// mean different things to a script: the containers finishing on their own is
/// success, the operator pressing Ctrl-C is not, a stream that died under a
/// container still running is a failed read, and an abort triggered by a
/// container exit carries the exit code the caller wants to propagate. The
/// caller still tears the project down in every case except the abort, where
/// `attach` itself already stopped every remaining container. Reporting an
/// ending as an error from `attach` would short-circuit that and leave the
/// containers running, which is a worse bug than the exit code this exists to
/// fix.
///
/// When the outcome is `Aborted`, the service name and exit code travel back
/// alongside it in [`AttachSummary`], not as fields of the variant: keeping the
/// enum all-unit preserves the `Copy` derive this enum had before
/// `--abort-on-container-exit` (#1492) and keeps the defined discriminants
/// every `as isize`/`mem::discriminant` caller relies on. The variant name was
/// the only piece of the struct that survived the rework; everything that
/// used to live inside the braces moved one level up.
///
/// `#[non_exhaustive]` since 3.0.0, so a further ending can be added without a
/// major bump. `StreamBroke` (3.3.0) and `Aborted` (#1492) are two that already
/// were.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachOutcome {
	/// Every stream ended on its own.
	StreamsEnded,
	/// SIGINT or SIGTERM arrived while streaming.
	Interrupted,
	/// At least one stream ended while its container was still running, so live
	/// output was truncated rather than finished (#1104).
	StreamBroke,
	/// `up --abort-on-container-exit` (or `--exit-code-from`, which implies it)
	/// fired: the first container to exit triggered the abort, the rest were
	/// stopped, and the exit code is what the CLI propagates as its process
	/// exit status. The service name and exit code are carried in the
	/// sibling [`AttachSummary::service`] / [`AttachSummary::exit_code`]
	/// fields, not as struct fields here, so the enum stays all-unit and the
	/// `Copy` derive survives.
	Aborted,
}

/// Options that go with [`Engine::attach_logs_with`]: every flag `podup up`
/// exposes that affects what an attached stream does or how it ends.
///
/// `#[non_exhaustive]` since 4.1.0, so the next flag is not a breaking change
/// for anyone building one with a literal. [`AttachOptions::default`] builds
/// the no-op set, and the rest of the surface is the `with_*` builders below.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AttachOptions {
	/// Prefix each streamed line with the libpod RFC3339 timestamp
	/// (`up --timestamps`).
	pub timestamps: bool,
	/// Stop the rest of the project as soon as any container exits
	/// (`up --abort-on-container-exit`). See [`AttachOutcome::Aborted`] for
	/// what the caller is then expected to do.
	pub abort_on_container_exit: bool,
	/// When set, the exit code reported back is the named service's code
	/// rather than the first container's. Implies `abort_on_container_exit`
	/// (matching `docker compose` v5.1.3): the abort has to fire to learn a
	/// later exit code, so passing only `--exit-code-from` is enough.
	pub exit_code_from: Option<String>,
}

impl AttachOptions {
	/// The no-options set: every flag off, `--timestamps` off, no named exit
	/// source. Equivalent to [`AttachOptions::default`].
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Set [`AttachOptions::timestamps`]. Builder-style.
	#[must_use]
	pub fn with_timestamps(mut self, timestamps: bool) -> Self {
		self.timestamps = timestamps;
		self
	}

	/// Set [`AttachOptions::abort_on_container_exit`]. Builder-style.
	#[must_use]
	pub fn with_abort_on_container_exit(mut self, abort_on_container_exit: bool) -> Self {
		self.abort_on_container_exit = abort_on_container_exit;
		self
	}

	/// Set [`AttachOptions::exit_code_from`]. Builder-style. The named
	/// service is checked against the compose file in
	/// [`Engine::attach_logs_with`], not here, so the validation error
	/// surfaces as `ServiceNotFound` from the call itself.
	#[must_use]
	pub fn with_exit_code_from(mut self, exit_code_from: Option<String>) -> Self {
		self.exit_code_from = exit_code_from;
		self
	}
}

/// What an attached `up` returned, pairing the always-known [`AttachOutcome`]
/// with the abort-specific extras that previously lived as fields of the
/// `Aborted` variant.
///
/// `service` and `exit_code` are only meaningful when `outcome` is
/// [`AttachOutcome::Aborted`]; they are `None` for the other three endings.
/// The two-abort-fields split off into this struct is what keeps the enum
/// `Copy` (and keeps its discriminant values defined); putting a `String`
/// next to an `i64` inside the variant would have broken both, and the
/// caller still needs to read them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachSummary {
	/// Which of the four endings the attach call reached.
	pub outcome: AttachOutcome,
	/// The compose service whose exit code is being propagated, on the
	/// `Aborted` path. `None` for every other ending.
	pub service: Option<String>,
	/// The exit code reported by `service`'s container, after the abort
	/// stopped everything. May be `0` (a clean exit), non-zero (a crash),
	/// or `137` (SIGKILL during the abort teardown when the named service
	/// was still running). `None` for every non-`Aborted` ending.
	pub exit_code: Option<i64>,
}

impl Engine {
	/// Attach to log streams for all services with `attach: true` (the default). Streams are multiplexed to stdout with a service-name prefix.
	pub async fn attach_logs(&self, file: &ComposeFile) -> Result<AttachOutcome> {
		self.attach_logs_with_options(file, false).await
	}

	/// Like [`Engine::attach_logs`] but with `up --timestamps` support.
	///
	/// `timestamps` prefixes each streamed line with the libpod RFC3339
	/// timestamp. This is the legacy two-parameter form kept verbatim for
	/// 4.0.0 callers; the abort-related flags moved to
	/// [`Engine::attach_logs_with`] so they could be added without breaking
	/// this signature. A caller that does not need the abort path can keep
	/// using this method and see no behaviour change between 4.0.0 and
	/// 4.1.0.
	pub async fn attach_logs_with_options(
		&self,
		file: &ComposeFile,
		timestamps: bool,
	) -> Result<AttachOutcome> {
		// Delegate with the abort set fully off, so the call path the rest of
		// the 4.0.0 callers still take is exercised by the wrapper itself,
		// not just by the underlying method when the new flags are turned on.
		self.attach_logs_with(
			file,
			&AttachOptions {
				timestamps,
				abort_on_container_exit: false,
				exit_code_from: None,
			},
		)
		.await
		.map(|summary| summary.outcome)
	}
}

#[cfg(test)]
#[path = "../attach_outcome_tests.rs"]
mod attach_outcome_tests;
