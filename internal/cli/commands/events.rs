//! Arguments of `events`: which slice of the Podman event stream is replayed,
//! and how each event is rendered.

use clap::Args;

use crate::cli::EventsFormat;

#[derive(Args)]
pub(crate) struct EventsArgs {
	/// Output format: a `TYPE ACTION NAME` summary (table) or one JSON
	/// object per line (NDJSON, not a JSON array).
	#[arg(long, value_enum, default_value_t = EventsFormat::Table)]
	pub(crate) format: EventsFormat,
	/// Only stream events at or after this timestamp/relative time (e.g.
	/// 30m, 2026-01-01T00:00:00). A relative time counts back from now:
	/// `30m` is thirty minutes ago.
	// `allow_hyphen_values` so a leading `-` (a time in the future, which
	// matches nothing, #1896) reaches podup's own error rather than clap's
	// unknown-flag one. A plain comment, not a doc comment, so it stays out
	// of `--help`.
	#[arg(long, allow_hyphen_values = true)]
	pub(crate) since: Option<String>,
	/// Only stream events up to this timestamp/relative time. Only an
	/// already-elapsed window ends the feed: libpod keeps a stream open past
	/// a future time, so `--until -5m` bounds it and `--until 5m` does not.
	#[arg(long, allow_hyphen_values = true)]
	pub(crate) until: Option<String>,
	/// Filter events by predicate (KEY=VALUE, e.g. event=start); repeatable.
	#[arg(long)]
	pub(crate) filter: Vec<String>,
	/// Deprecated alias for `--format json`; kept for backward compatibility.
	/// Cannot be combined with an explicit `--format`.
	#[arg(long, hide = true, conflicts_with = "format")]
	pub(crate) json: bool,
	/// Only stream events for these services (all when none are given).
	// A plain positional (no `trailing_var_arg`), like `top`, so `--format`,
	// `--since`, `--until`, and `--filter` parse in any position
	// (`events web db --format json` as well as `events --format json
	// web db`); service names are never hyphen-prefixed, so nothing is
	// lost. An unknown service fails with
	// `service 'X' not found` before the event stream is opened (#2014).
	pub(crate) services: Vec<String>,
}
