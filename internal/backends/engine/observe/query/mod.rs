//! Query and observation commands: ps, logs, exec, pull, remove_orphans.

use crate::compose::types::ComposeFile;
use crate::error::Result;

use crate::engine::Engine;

mod attach;
mod listing;
mod logs;
mod options;
mod session;

use listing::inspect;
use listing::ps;
use session::exec;
pub(crate) use session::terminal;

#[cfg(test)]
pub(crate) use ps::ps_rows_for_test;
pub use ps::{PsDisplayOptions, PsFilterOptions, PsOptions};

pub use exec::ExecOptions;
pub(crate) use exec::InputSource;
pub use options::{ImagesOptions, LogsDisplay, LogsOptions, DEFAULT_LOG_TAIL};

pub use attach::{AttachOptions, AttachOutcome, AttachSummary};

// Re-exported so callers in `engine::*` (`lifecycle/run/mod.rs`,
// `query/attach.rs`) reach it via the same path they used when the helper
// lived in `query/mod.rs`. The item is `pub(in crate::backends::engine)` in
// `logs/stream.rs`; the re-export documents the reach and keeps `mod.rs` as
// the canonical place to look for engine query commands.
pub(in crate::backends::engine) use logs::stream_broke_mid_output;

impl Engine {
	/// Stream logs. When `service_name` is `None`, streams from all services. When `follow` is true, tails indefinitely.
	pub async fn logs(
		&self,
		file: &ComposeFile,
		service_name: Option<&str>,
		follow: bool,
	) -> Result<()> {
		let targets: Vec<String> = service_name
			.map(|s| vec![s.to_string()])
			.unwrap_or_default();
		self.logs_with_options(
			file,
			&targets,
			LogsOptions {
				follow,
				..Default::default()
			},
		)
		.await
	}

	/// Stream logs with `docker compose logs` options (`--tail`, `--since`,
	/// `--until`, `--timestamps`, `--follow`). For the `--no-color`/
	/// `--no-log-prefix` prefix-display options use [`Engine::logs_with_display`].
	///
	/// When `target_services` is empty, logs from every service are streamed;
	/// otherwise only the named services (an unknown name is an error).
	pub async fn logs_with_options(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		opts: LogsOptions,
	) -> Result<()> {
		self.logs_with_display(file, target_services, opts, LogsDisplay::default())
			.await
	}
}

#[cfg(test)]
mod attach_stream_tests;
