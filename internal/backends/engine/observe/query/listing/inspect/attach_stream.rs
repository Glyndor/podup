//! The `attach` command's stream: how a frame reaches the caller's streams.
//!
//! `attach` is output-only: it hands the container's stdout and stderr to the
//! caller and stops when the container does. Both halves of that are decided
//! here, away from the HTTP glue in the parent module, so each is testable on
//! its own.

use std::io::Write;

use crate::libpod::LogOutput;

/// Write one frame of an attached container's output to the caller's streams.
///
/// The payload is bytes, not text: what the container printed is what the
/// caller gets. Each frame used to be converted on its own, which replaced
/// every byte that is not UTF-8 with U+FFFD and turned a character that
/// arrived split across two frames into two replacement characters.
pub(super) fn write_attach_frame<O: Write, E: Write>(
	out: &mut O,
	err: &mut E,
	frame: LogOutput,
) -> std::io::Result<()> {
	match frame {
		LogOutput::StdOut { message } => out.write_all(&message),
		LogOutput::StdErr { message } => err.write_all(&message),
	}
}

#[cfg(test)]
#[path = "attach_stream_tests.rs"]
mod tests;
