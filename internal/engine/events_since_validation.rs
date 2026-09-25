//! `events --since` validator.
//!
//! Lifted from `events.rs` so the orchestration in that file stays
//! under the 500-line source budget; the validator is a pure function
//! with its own concerns (a hand-written Go duration matcher that
//! rejects only negative relative times and forwards everything else
//! to libpod unchanged). The tests live next to the orchestration
//! file (`events_tests.rs`) and reach the validator through
//! `super::since_validation`.

use crate::error::{ComposeError, Result};

/// Reject a `--since` written as a negative relative duration.
///
/// libpod reads a relative `since` as a time before now, so `30m` is thirty
/// minutes ago and `-30m` is thirty minutes in the future: a window that
/// starts there matches nothing and the feed looks empty (#1896). A plain
/// negative number is left alone, though: libpod's `ParseInputTime` parses
/// numeric values as Unix timestamps before trying them as durations, so a
/// pre-epoch lower bound like `--since -1` is a valid replay window, and
/// `-0s`/`-0m` are zero offsets (i.e. "now"). The check matches Go's
/// duration syntax exactly on the part after `-`: one or more segments,
/// each a number immediately followed by a unit from `ns`, `us`, `µs`,
/// `ms`, `s`, `m`, `h`, with at least one non-zero digit overall. Anything
/// that fails to parse that way is forwarded unchanged so `1e3`, negative
/// timestamps and zero offsets still reach libpod.
pub(crate) fn validate_events_since(since: Option<&str>) -> Result<()> {
	let Some(v) = since else {
		return Ok(());
	};
	let Some(rest) = v.strip_prefix('-') else {
		return Ok(());
	};
	if is_go_duration(rest) {
		return Err(ComposeError::Unsupported(format!(
			"invalid --since value {v:?}: a relative time counts back from now, so write it without the leading '-' (e.g. --since {rest})"
		)));
	}
	Ok(())
}

/// Exact Go duration syntax for the part after a leading `-`. Returns
/// `true` only when `rest` is one or more segments, each a number (`123`,
/// `1.5`, `.5`, `1.`) immediately followed by a unit from `ns`, `us`,
/// `µs`, `ms`, `s`, `m`, `h`, with no trailing characters and at least one
/// non-zero digit overall. Used only by [`validate_events_since`].
///
/// Hand-written on purpose: the inputs are short and a regex crate would
/// be heavier than the parser it would replace. Kept as small as the
/// surface it has to cover.
fn is_go_duration(rest: &str) -> bool {
	let mut chars = rest.chars().peekable();
	let mut has_non_zero_overall = false;

	loop {
		let mut saw_digit = false;
		let mut segment_has_non_zero = false;

		while let Some(&c) = chars.peek() {
			if c.is_ascii_digit() {
				chars.next();
				saw_digit = true;
				if c != '0' {
					segment_has_non_zero = true;
				}
			} else {
				break;
			}
		}

		if chars.peek() == Some(&'.') {
			chars.next();
			let mut frac_has_digit = false;
			while let Some(&c) = chars.peek() {
				if c.is_ascii_digit() {
					chars.next();
					frac_has_digit = true;
					if c != '0' {
						segment_has_non_zero = true;
					}
				} else {
					break;
				}
			}
			// Go's `time.ParseDuration` accepts `.5` and `1.`, but not a bare
			// `.`. A segment needs at least one digit on one side of the dot.
			if !saw_digit && !frac_has_digit {
				return false;
			}
			saw_digit = saw_digit || frac_has_digit;
		}

		if !saw_digit {
			// No number for the unit to attach to.
			return false;
		}

		// A unit must follow the number; a bare `5` without a unit is not a
		// Go duration.
		match chars.next() {
			Some('n') | Some('u') | Some('\u{00B5}') => {
				if chars.peek() != Some(&'s') {
					return false;
				}
				chars.next();
			}
			Some('m') => {
				if chars.peek() == Some(&'s') {
					chars.next();
				}
			}
			Some('s') | Some('h') => {}
			_ => return false,
		}

		if segment_has_non_zero {
			has_non_zero_overall = true;
		}

		if chars.peek().is_none() {
			break;
		}
	}

	has_non_zero_overall
}
