//! Prints attached `up` output with a `{service} | ` tag, decoding each
//! channel incrementally so a character split across two frames is not
//! corrupted.

use super::super::utf8::Utf8Chunks;

/// Writes the frames of one container's stream.
///
/// Stdout and stderr each keep their own decoder: a character is never split
/// across the two channels, but each channel's frames can split one.
pub(super) struct ChunkPrinter<'a> {
	prefix: &'a str,
	/// Print the text alone, without the `{prefix} | ` tag (`--no-log-prefix`).
	plain: bool,
	out: Utf8Chunks,
	err: Utf8Chunks,
}

impl<'a> ChunkPrinter<'a> {
	pub(super) fn new(prefix: &'a str) -> Self {
		Self {
			prefix,
			plain: false,
			out: Utf8Chunks::default(),
			err: Utf8Chunks::default(),
		}
	}

	/// Print the text without the service tag.
	pub(super) fn plain(mut self, plain: bool) -> Self {
		self.plain = plain;
		self
	}

	fn tagged(&self, text: String) -> Option<String> {
		(!text.is_empty()).then(|| super::follow::render_line(self.prefix, self.plain, &text))
	}

	/// The text to print for a stdout frame, or `None` while the frame holds
	/// only the start of a character.
	pub(super) fn stdout_text(&mut self, frame: &[u8]) -> Option<String> {
		let text = self.out.push(frame);
		self.tagged(text)
	}

	/// The text to print for a stderr frame. See [`Self::stdout_text`].
	pub(super) fn stderr_text(&mut self, frame: &[u8]) -> Option<String> {
		let text = self.err.push(frame);
		self.tagged(text)
	}

	/// Print a stdout frame. A closed pipe ends the process quietly; any other
	/// write error comes back so the caller can end the stream.
	pub(super) fn stdout(&mut self, frame: &[u8]) -> std::io::Result<()> {
		match self.stdout_text(frame) {
			Some(text) => crate::ui::out!("{text}"),
			None => Ok(()),
		}
	}

	pub(super) fn stderr(&mut self, frame: &[u8]) -> std::io::Result<()> {
		match self.stderr_text(frame) {
			Some(text) => crate::ui::err!("{text}"),
			None => Ok(()),
		}
	}

	/// Print any character left unfinished when the stream ended. It shows as
	/// `U+FFFD`, since its remaining bytes will never arrive.
	pub(super) fn finish(&mut self) -> std::io::Result<()> {
		let rest = self.out.finish();
		if let Some(text) = self.tagged(rest) {
			crate::ui::out!("{text}")?;
		}
		let rest = self.err.finish();
		if let Some(text) = self.tagged(rest) {
			crate::ui::err!("{text}")?;
		}
		Ok(())
	}
}

#[cfg(test)]
#[path = "printer_tests.rs"]
mod tests;
