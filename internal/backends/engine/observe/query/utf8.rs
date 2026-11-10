//! Keeping a multibyte character whole when a byte stream is cut into chunks.
//!
//! A container's output arrives as frames whose boundaries are chosen by the
//! engine and by our own buffering, not by the text. A character of two, three
//! or four bytes can therefore be split between two frames, and decoding each
//! frame on its own turns both halves into replacement characters. These
//! helpers hold the unfinished tail of a chunk back until the rest arrives.

/// How many bytes at the end of `bytes` are the start of a UTF-8 character that
/// the slice does not finish yet (0 to 3).
///
/// Only a tail that could still become a valid character counts. Bytes that can
/// never be completed (a stray continuation byte, a lead byte followed by
/// something that is not a continuation) are not held back, so a stream of
/// genuinely invalid bytes still decodes to replacement characters instead of
/// waiting forever.
pub(super) fn incomplete_tail_len(bytes: &[u8]) -> usize {
	let n = bytes.len();
	for back in 1..=n.min(3) {
		let b = bytes[n - back];
		if b & 0xC0 == 0x80 {
			continue;
		}
		let needed = match b {
			0xC2..=0xDF => 2,
			0xE0..=0xEF => 3,
			0xF0..=0xF4 => 4,
			_ => return 0,
		};
		let tail = &bytes[n - back..];
		return if tail.len() < needed && is_valid_prefix(tail) {
			back
		} else {
			0
		};
	}
	0
}

/// Whether `tail` (a lead byte plus any continuations) can still grow into a
/// valid character. A short tail is valid exactly when `from_utf8` fails with
/// "unexpected end of input" and not with an invalid sequence.
fn is_valid_prefix(tail: &[u8]) -> bool {
	matches!(std::str::from_utf8(tail), Err(e) if e.error_len().is_none())
}

/// Decodes a stream of chunks into text without splitting a character that
/// straddles two chunks.
///
/// Bytes that are not valid UTF-8 become `U+FFFD`, exactly as
/// [`String::from_utf8_lossy`] would for the whole stream.
#[derive(Default)]
pub(super) struct Utf8Chunks {
	carry: Vec<u8>,
}

impl Utf8Chunks {
	/// Decode `chunk`, holding back a trailing incomplete character for the next
	/// call.
	pub(super) fn push(&mut self, chunk: &[u8]) -> String {
		self.carry.extend_from_slice(chunk);
		let keep = incomplete_tail_len(&self.carry);
		let cut = self.carry.len() - keep;
		let text = String::from_utf8_lossy(&self.carry[..cut]).into_owned();
		self.carry.drain(..cut);
		text
	}

	/// Decode whatever is still held back. Call once when the stream ends: a
	/// character that never got its remaining bytes is a real truncation and
	/// shows as `U+FFFD`.
	pub(super) fn finish(&mut self) -> String {
		let text = String::from_utf8_lossy(&self.carry).into_owned();
		self.carry.clear();
		text
	}
}

#[cfg(test)]
#[path = "utf8_tests.rs"]
mod tests;
