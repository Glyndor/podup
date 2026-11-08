//! Bounded echo of untrusted text in an error message.

use super::error::sanitize_name;

/// The most characters of an untrusted value an error message repeats.
pub(crate) const MAX_ECHOED_CHARS: usize = 80;

/// `s` as an error message may show it: at most [`MAX_ECHOED_CHARS`]
/// characters, control characters escaped as [`sanitize_name`] does, and when
/// the text was cut an ellipsis with the number of characters left out, so a
/// multi-megabyte value in a compose file cannot become a multi-megabyte
/// error line. The cut counts characters, never bytes, so it cannot split a
/// multibyte one.
pub(crate) fn clip_for_message(s: &str) -> String {
	let Some((cut, _)) = s.char_indices().nth(MAX_ECHOED_CHARS) else {
		return sanitize_name(s).into_owned();
	};
	let omitted = s[cut..].chars().count();
	format!("{}...({omitted} more characters)", sanitize_name(&s[..cut]))
}
