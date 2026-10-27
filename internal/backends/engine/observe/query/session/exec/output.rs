//! Writing an exec session's output to the caller's streams.
//!
//! The payload of an output frame is bytes, not text: what the command
//! printed is what the caller gets. Both writers here therefore hand the
//! bytes over untouched, so a session that pipes a compressed archive to
//! stdout delivers the archive and a character that arrived split across two
//! frames arrives whole.

use std::io::Write;

use bytes::Bytes;

use crate::libpod::LogOutput;

use super::is_exec_teardown_noise;

/// What one output frame of an exec session means.
pub(crate) enum FrameAction {
	/// The bytes belong on the caller's stdout.
	StdOut(Bytes),
	/// The bytes belong on the caller's stderr.
	StdErr(Bytes),
	/// The frame is the spurious teardown noise and is dropped.
	Drop,
}

/// Decide what an output frame means, without writing it.
///
/// One filter applies, and it applies to stderr only: the secondary
/// `read unixpacket ... connection reset by peer` frame an OCI exec launch
/// failure emits adds nothing to the real diagnostic, so it is dropped
/// rather than shown.
pub(crate) fn frame_action(frame: LogOutput) -> FrameAction {
	match frame {
		LogOutput::StdOut { message } => FrameAction::StdOut(message),
		LogOutput::StdErr { message } => {
			if is_exec_teardown_noise(&String::from_utf8_lossy(&message)) {
				return FrameAction::Drop;
			}
			FrameAction::StdErr(message)
		}
	}
}

/// Write one output frame to the caller's streams.
pub(crate) fn write_exec_frame<O: Write, E: Write>(
	out: &mut O,
	err: &mut E,
	frame: LogOutput,
) -> std::io::Result<()> {
	match frame_action(frame) {
		FrameAction::StdOut(bytes) => out.write_all(&bytes),
		FrameAction::StdErr(bytes) => err.write_all(&bytes),
		FrameAction::Drop => Ok(()),
	}
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;
