//! Assembling one frame's rows and deciding where they go.
//!
//! The rows a frame carries are the live samples for the running containers
//! plus, under `--all`, a zeroed row for each container that is not running.
//! What happens to those rows is a separate question from how a single row is
//! spelled: the table repaints in place only while streaming and only on a
//! terminal, and the machine path prints a document instead.

use std::collections::HashSet;

use crate::error::{ComposeError, Result};

use super::model::{ContainerStat, StatsReport};
use super::options::StatsOptions;
use super::render::{format_row_with, header, stat_json_row};

/// Assemble the rows for one frame: the live samples for `running` containers
/// plus synthesized zero rows for each `stopped` container (already empty when
/// `--all` is off), sorted by name for stable output.
pub(super) fn frame_rows(
	report: &StatsReport,
	running: &HashSet<String>,
	stopped: &[String],
) -> Vec<ContainerStat> {
	let mut rows: Vec<ContainerStat> = report
		.stats
		.iter()
		.filter(|s| running.contains(&s.name))
		.cloned()
		.collect();
	for name in stopped {
		rows.push(ContainerStat {
			name: name.clone(),
			..ContainerStat::default()
		});
	}
	rows.sort_by(|a, b| a.name.cmp(&b.name));
	rows
}

/// Whether a streaming `stats` frame should repaint in place.
///
/// Pure, and separate from the terminal probe, so the `--format json` half is
/// testable: a piped test cannot exercise it at all, because the terminal probe
/// is already false there: a version that let JSON repaint on a tty passed
/// every integration test in the suite.
pub(super) fn wants_region(json: bool, live_terminal: bool) -> bool {
	// Never for the machine path, whatever the terminal says. A parser reading
	// NDJSON would otherwise get cursor moves interleaved with its documents.
	!json && live_terminal
}

/// Whether `stats` may repaint in place.
///
/// Asked of **stdout**, unlike the lifecycle board's stderr: `stats` renders its
/// table to stdout, so that is the stream that has to be a terminal for a
/// repaint to make sense. `stats > file` on a terminal must still produce a file
/// of readable frames rather than a file of cursor moves.
pub(super) fn live_stats_allowed() -> bool {
	use std::io::IsTerminal;
	std::io::stdout().is_terminal()
		&& crate::ui::stdout_colored()
		&& crate::engine::query::terminal::window_size().is_some()
}

/// Print one stats frame: the table (a bold header plus one row per container)
/// or a JSON array when `--format json`. Table frames end with a blank line.
pub(super) fn print_frame(
	report: &StatsReport,
	running: &HashSet<String>,
	stopped: &[String],
	opts: &StatsOptions,
	region: Option<&mut crate::ui::progress::Region>,
) -> Result<()> {
	let rows = frame_rows(report, running, stopped);

	if opts.json {
		let json: Vec<_> = rows.iter().map(stat_json_row).collect::<Result<Vec<_>>>()?;
		// While streaming, one compact array per line: NDJSON, the shape
		// `events` already emits. A pretty-printed array per frame, concatenated,
		// is neither a single JSON document nor NDJSON, so no parser accepts it:
		// `stats --format json` was unreadable by anything for as long as it
		// streamed. `--no-stream` prints one frame and exits, so it stays a
		// single pretty document, which is valid JSON and nicer to read.
		// Propagated, not swallowed. `unwrap_or_default()` printed an empty line
		// on a serialisation failure and carried on, so `stats --format json`
		// emitted a blank where a frame should be and still exited zero -- a
		// consumer piping into `jq` sees a gap and no reason for it. The other
		// five list commands route through `to_pretty_json`, whose contract says
		// a failure "must propagate as an error and exit non-zero"; stats was
		// the one that did not. Same error variant, so the message reads alike.
		let text = if opts.no_stream {
			serde_json::to_string_pretty(&json)
		} else {
			serde_json::to_string(&json)
		}
		.map_err(|e| ComposeError::Build(format!("invalid stats frame: {e}")))?;
		println!("{text}");
		return Ok(());
	}

	let colour = crate::ui::stdout_colored();
	let mut lines = vec![crate::ui::paint(crate::ui::bold(), &header(), colour)];
	for s in &rows {
		lines.push(format_row_with(s, opts.no_trunc, colour)?);
	}

	// A live region only while streaming. `--no-stream` prints one frame and
	// exits, so there is nothing to repaint over and a region would only hide
	// the cursor for no reason.
	if let Some(region) = region {
		region.show(&[], &lines);
		return Ok(());
	}
	for line in lines {
		println!("{line}");
	}
	println!();

	Ok(())
}
