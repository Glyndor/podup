//! The flags `stats` accepts.
//!
//! Kept apart from the command that reads them so the option surface can grow
//! a flag without the stream loop being read to find out how to spell one.

/// Options for
/// [`Engine::stats_with_options`](crate::engine::Engine::stats_with_options),
/// mirroring `docker compose stats` and the table-shaping flags the other list
/// commands expose.
///
/// `#[non_exhaustive]` since 4.0.0, so a new flag can be added in a minor
/// release without breaking every external caller that built the struct with
/// a literal. Construct it via [`StatsOptions::new`] or the `with_*` builders
/// below; a struct literal is refused outside this crate, which is what buys
/// the room to grow.
#[derive(Default)]
#[non_exhaustive]
pub struct StatsOptions {
	/// Disable streaming; print a single snapshot and exit, `--no-stream`.
	pub no_stream: bool,
	/// Include non-running containers as zeroed rows, `-a/--all`.
	pub all: bool,
	/// Emit JSON instead of the table, `--format json`.
	pub json: bool,
	/// Disable container-name truncation in the table, `--no-trunc`.
	pub no_trunc: bool,
}

impl StatsOptions {
	/// Build options from the four CLI flags, in `--no-stream`/`--all`/
	/// `--no-trunc`/`--format json` order. A terse constructor so the CLI keeps
	/// the field names (all `pub`) available for clarity while the dispatch site
	/// stays compact.
	pub fn new(no_stream: bool, all: bool, no_trunc: bool, json: bool) -> Self {
		Self {
			no_stream,
			all,
			no_trunc,
			json,
		}
	}

	/// Include non-running containers as zeroed rows, `-a/--all`. Builder-style.
	#[must_use]
	pub fn with_all(mut self, all: bool) -> Self {
		self.all = all;
		self
	}

	/// Emit JSON instead of the table, `--format json`. Builder-style.
	#[must_use]
	pub fn with_json(mut self, json: bool) -> Self {
		self.json = json;
		self
	}
}
