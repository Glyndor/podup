//! Turning one container's counters into a table row or a JSON object.
//!
//! Every function here is pure: given a sample, it returns the string a reader
//! sees, so the column layout can be checked without a terminal and the header
//! can be pinned against the rows it labels.

use crate::error::{ComposeError, Result};
use crate::units::SizeFormat;

use super::model::ContainerStat;

/// Width of the table NAME column; long names are truncated to this width (with
/// a trailing ellipsis) unless `--no-trunc` is given, so a long container name
/// no longer overflows and shifts every following column. The header is built
/// from this same constant, so the two cannot drift apart.
pub(super) const NAME_WIDTH: usize = 32;

/// How `stats` renders a size: binary units at one decimal.
///
/// Binary because this table is read next to `free` and `htop`, which are
/// binary, not next to podman's own output, which is decimal. One decimal
/// because the columns below are fixed-width by design (a live view must not
/// resize itself mid-repaint), and a second decimal does not fit `NET I/O`.
const SIZE_FORMAT: SizeFormat = SizeFormat::binary().with_decimals(1);

/// Render a byte count as a compact human string (`1.5MiB`). Pure for testing.
///
/// The ladder, the rounding and the totality guarantee live in [`crate::units`]
/// now; this is the surface's choice of base and precision, nothing else.
pub(super) fn format_bytes(bytes: u64) -> String {
	crate::units::format_bytes(bytes, &SIZE_FORMAT)
}

/// Sum a container's per-interface network counters into one `(rx, tx)` pair.
///
/// Checked, because the daemon chooses both the numbers and how many
/// interfaces there are: each counter is a `u64`, so two of them can add up to
/// more than a `u64` holds. The sum was unchecked, and an overflow panics in a
/// debug build and wraps in a release one, so the total the table (and the
/// JSON) showed was not the traffic that happened. A sum that does not fit is
/// reported instead of guessed.
fn net_totals(s: &ContainerStat) -> Result<(u64, u64)> {
	let mut rx = 0u64;
	let mut tx = 0u64;
	for n in s.network.values() {
		rx = rx
			.checked_add(n.rx)
			.ok_or_else(|| ComposeError::StatsOverflow(s.name.clone()))?;
		tx = tx
			.checked_add(n.tx)
			.ok_or_else(|| ComposeError::StatsOverflow(s.name.clone()))?;
	}
	Ok((rx, tx))
}

/// The NAME cell for the table: the full name when `no_trunc`, otherwise
/// truncated to [`NAME_WIDTH`] with a trailing ellipsis so a long name keeps the
/// row aligned. Counts characters (not bytes) so multi-byte names truncate
/// safely. Pure for testing.
pub(super) fn truncate_name(name: &str, no_trunc: bool) -> String {
	if no_trunc || name.chars().count() <= NAME_WIDTH {
		return name.to_string();
	}
	let head: String = name.chars().take(NAME_WIDTH - 1).collect();
	format!("{head}…")
}

/// Format one stats row into the table layout. With `no_trunc` a long name is
/// left intact (and may overflow its column); otherwise it is truncated to
/// [`NAME_WIDTH`]. Pure for testing.
/// Colour band for a utilisation percentage.
///
/// A container at 95% of its memory limit is minutes from being OOM-killed, and
/// in a plain table that number looks exactly like 0.02%. The whole reason to
/// read `stats` is to find the row that is in trouble, so the number that says
/// so should be the one that catches the eye.
pub(super) fn load_style(pct: f64) -> crate::ui::Style {
	use crate::ui::AnsiColor;
	// Four bands, not three, and the lowest one is dim rather than green.
	// Everything under 70% used to be the same green, so a container at 0.02%
	// looked exactly like one at 69%: colour that does not vary with the value
	// carries no information, and the whole reason to read `stats` is to find
	// the row in trouble. Idle rows now recede instead of competing.
	if pct < 5.0 {
		return crate::ui::Style::new().dimmed();
	}
	let colour = if pct >= 85.0 {
		AnsiColor::Red
	} else if pct >= 50.0 {
		AnsiColor::Yellow
	} else {
		AnsiColor::Green
	};
	crate::ui::Style::new().fg_color(Some(colour.into()))
}

/// Format one stats row into the table layout, optionally coloured. With
/// `no_trunc` a long name is left intact (and may overflow its column);
/// otherwise it is truncated to [`NAME_WIDTH`]. Pure, so the layout is testable
/// without a terminal; the tests pass `colour = false`.
///
/// Each cell is padded to its width *before* being painted: the ANSI codes are
/// zero-width, so padding afterwards would count them and knock every later
/// column out of alignment.
pub(super) fn format_row_with(s: &ContainerStat, no_trunc: bool, colour: bool) -> Result<String> {
	use crate::ui::{identity_style, paint};
	let (rx, tx) = net_totals(s)?;
	let dim = crate::ui::Style::new().dimmed();

	// The name arrives from the daemon, so it is not ours to trust: a raw
	// escape in it repaints the reader's terminal and desynchronises the
	// resets of the colours painted beside it. Escaping happens before the
	// truncation and the padding, so the width the column reserves is the
	// width actually printed.
	let safe_name = crate::ui::sanitize_cell(s.name.trim());
	let name = format!("{:<NAME_WIDTH$}", truncate_name(&safe_name, no_trunc));
	let name = paint(identity_style(&safe_name), &name, colour);
	let cpu = paint(
		load_style(s.cpu),
		&format!("{:>width$.2}%", s.cpu, width = CPU_WIDTH - 1),
		colour,
	);
	let mem_pct = paint(
		load_style(s.mem_perc),
		&format!("{:>width$.2}%", s.mem_perc, width = MEM_PCT_WIDTH - 1),
		colour,
	);
	// Secondary detail: the absolute figures matter once a percentage has drawn
	// you to the row, not before.
	let mem = paint(
		dim,
		&format!(
			"{:>width$} / {:<width$}",
			format_bytes(s.mem_usage),
			format_bytes(s.mem_limit),
			width = (MEM_WIDTH - 3) / 2
		),
		colour,
	);
	let net = paint(
		dim,
		&format!(
			"{:>width$} / {:<width$}",
			format_bytes(rx),
			format_bytes(tx),
			width = (NET_WIDTH - 3) / 2
		),
		colour,
	);
	let block = paint(
		dim,
		&format!(
			"{:>width$} / {:<width$}",
			format_bytes(s.block_in),
			format_bytes(s.block_out),
			width = (BLOCK_WIDTH - 3) / 2
		),
		colour,
	);
	// PIDS was the only unstyled column of the six. It is secondary detail like
	// the absolute byte figures beside it, so it takes the same dim treatment
	// rather than a colour of its own.
	let pids = paint(dim, &format!("{:>PIDS_WIDTH$}", s.pids), colour);
	Ok(format!("{name} {cpu} {mem} {mem_pct} {net} {block} {pids}"))
}

/// Build one `stats --format json` row with numeric values (raw bytes/percent),
/// so machine consumers get exact figures rather than the table's rounded,
/// human-formatted cells. Pure so it can be unit-tested.
pub(super) fn stat_json_row(s: &ContainerStat) -> Result<serde_json::Value> {
	let (rx, tx) = net_totals(s)?;
	Ok(serde_json::json!({
		"Name": s.name,
		"CPUPerc": s.cpu,
		"MemUsage": s.mem_usage,
		"MemLimit": s.mem_limit,
		"MemPerc": s.mem_perc,
		"NetInput": rx,
		"NetOutput": tx,
		"BlockInput": s.block_in,
		"BlockOutput": s.block_out,
		"PIDs": s.pids,
	}))
}

/// Width of each column after NAME, in the order they are printed.
///
/// One source for the header and the rows. They used to be two hand-maintained
/// layouts that nothing checked against each other, and they had drifted:
/// measured against a representative row, the `MEM %` label sat one column past
/// where its own data ended and `PIDS` started exactly where its data *stopped*,
/// so the label was entirely off its column. Patching the constant would have
/// fixed it until the next edit.
const CPU_WIDTH: usize = 8;
const MEM_WIDTH: usize = 23;
const MEM_PCT_WIDTH: usize = 7;
const NET_WIDTH: usize = 21;
const BLOCK_WIDTH: usize = 21;
const PIDS_WIDTH: usize = 5;

/// The table header, built from the same widths the rows are.
///
/// Fixed widths rather than `ui::Table`'s content sizing, deliberately: this
/// table repaints every second, and a column that resizes itself as the numbers
/// change makes every row jump sideways while you are trying to read it. The
/// shared table is right for a list printed once; a live view wants columns that
/// stay put.
pub(super) fn header() -> String {
	format!(
		"{:<NAME_WIDTH$} {:>CPU_WIDTH$} {:<MEM_WIDTH$} {:>MEM_PCT_WIDTH$} \
		 {:<NET_WIDTH$} {:<BLOCK_WIDTH$} {:>PIDS_WIDTH$}",
		"NAME", "CPU %", "MEM USAGE / LIMIT", "MEM %", "NET I/O", "BLOCK I/O", "PIDS"
	)
}
