//! Status styling: what colour a container state, a systemd state or an event
//! verb wears.
//!
//! Semantic, and the counterpart to the identity palette: green is up/healthy,
//! red is failed/dead, yellow is transient, dim is created or finished cleanly.
//! A word with no status meaning is left alone rather than given an arbitrary
//! colour, because a colour here claims a meaning it does not have.

use super::paint;
use super::AnsiColor;
use super::Style;

/// Whether an `Exited (N)` / `exited(N)` label reports a clean finish.
///
/// A container that ran to completion is not a failure, and colouring it red
/// says it is. One-shot services (migrations, seeds, a `command` that simply
/// ends) spend their whole life in this state, so red here is not a rare
/// cosmetic slip but the normal reading of a healthy project.
fn is_clean_exit(lower: &str) -> bool {
	// `exited (0)` and `exited(0)`, but not `exited (07)` or `exited (10)`.
	let Some(rest) = lower
		.split_once("exit")
		.map(|(_, r)| r.trim_start_matches(['e', 'd', ' ', '(']))
	else {
		return false;
	};
	rest.starts_with('0')
		&& !rest
			.trim_start_matches('0')
			.starts_with(|c: char| c.is_ascii_digit())
}

/// The semantic colour for a container status word, or `None` for an unknown one
/// (left uncoloured). Green = up/healthy, red = failed/dead/unhealthy, yellow =
/// paused/(re)starting/stopping, dim = created or finished cleanly. Matches on
/// substrings so it handles the bare state (`running`), Podman's verbose
/// `Status` (`Up 2 minutes`, `Exited (1)`), and systemd's vocabulary, which
/// `autostart status` reports.
pub(super) fn status_style(status: &str) -> Option<Style> {
	let s = status.to_ascii_lowercase();
	// Checked before the red arm: `Exited (0)` contains "exit" and would
	// otherwise be indistinguishable from `Exited (7)`.
	if s.contains("exit") && is_clean_exit(&s) {
		return Some(Style::new().dimmed());
	}
	let colour = if s.contains("unhealthy")
		|| s.contains("exit")
		|| s.contains("dead")
		|| s.contains("failed")
		|| s.contains("not-found")
		|| s.contains("error")
	{
		AnsiColor::Red
	} else if s.contains("running")
		|| s.contains("healthy")
		|| s.starts_with("up")
		|| s == "active"
		|| s == "enabled"
		|| s == "yes"
	{
		AnsiColor::Green
	} else if s.contains("paus")
		|| s.contains("restart")
		|| s.contains("starting")
		|| s.contains("stopping")
		|| s.contains("removing")
		|| s.contains("activating")
	{
		AnsiColor::Yellow
	} else if s.contains("created") || s == "inactive" || s == "disabled" || s == "no" {
		return Some(Style::new().dimmed());
	} else {
		return None;
	};
	Some(Style::new().fg_color(Some(colour.into())))
}

/// The colour for an event action or a container state word, whichever matches.
///
/// `events` streams verbs (`start`, `die`, `kill`, `health_status`) that are not
/// container states but mean the same kinds of thing, so they resolve through
/// the lifecycle bands first and fall back to the status vocabulary.
pub fn action_or_status_style(word: &str) -> Option<Style> {
	let w = word.to_ascii_lowercase();
	if w.starts_with("die")
		|| w.starts_with("kill")
		|| w.starts_with("destroy")
		|| w.starts_with("remove")
	{
		return Some(Style::new().fg_color(Some(AnsiColor::Red.into())));
	}
	if w.starts_with("start") || w.starts_with("create") || w.starts_with("health") {
		return Some(Style::new().fg_color(Some(AnsiColor::Green.into())));
	}
	if w.starts_with("stop") || w.starts_with("pause") || w.starts_with("restart") {
		return Some(Style::new().fg_color(Some(AnsiColor::Yellow.into())));
	}
	status_style(word)
}

/// Colourise a status cell, tinting each comma-separated segment on its own.
///
/// `ls` reports a project as `running(1), exited(1)`, and styling that as one
/// string made the first matching substring win: `exit` came first, so a project
/// with a service up rendered **entirely red**, visually identical to one that
/// is completely dead. Splitting first means each state carries its own colour
/// and the mixed case reads as mixed.
///
/// Trailing padding is preserved verbatim so column alignment is untouched.
pub(crate) fn paint_status_cell(padded: &str) -> String {
	let trimmed = padded.trim_end();
	let pad = &padded[trimmed.len()..];
	let body = trimmed
		.split(", ")
		.map(|seg| match status_style(seg) {
			Some(style) => paint(style, seg, true),
			None => seg.to_string(),
		})
		.collect::<Vec<_>>()
		.join(", ");
	format!("{body}{pad}")
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
