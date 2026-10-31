//! Where an attached session reads the caller's standard input from.
//!
//! A session that carries the caller's stdin has to read it from somewhere,
//! and the somewhere is not always the process's own stdin: a test cannot
//! use the terminal attached to the test runner (it would block, or steal
//! the developer's keystrokes).

use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, ReadBuf};

use super::stdin_is_terminal;

/// The source an attached session reads the caller's standard input from.
#[derive(Debug, Default)]
pub(crate) enum InputSource {
	/// The process's own standard input.
	#[default]
	Process,
	/// A fixed buffer of bytes. It is read to its end and then closed, which
	/// is what a redirected file looks like to the session.
	///
	/// Only the tests build this: the production path has exactly one
	/// source, the process's stdin, and a second variant nobody constructs
	/// would only be dead code.
	#[cfg(test)]
	Buffer(Vec<u8>),
}

impl InputSource {
	/// Whether the caller's input is a terminal.
	///
	/// A terminal is not standard input to hand to a command: the command
	/// would wait for input that a redirected run never provides, and the
	/// pseudo-terminal is the only shape in which a terminal is the right
	/// thing to pass on.
	pub(crate) fn is_terminal(&self) -> bool {
		match self {
			InputSource::Process => stdin_is_terminal(),
			#[cfg(test)]
			InputSource::Buffer(_) => false,
		}
	}

	/// The reader this source hands to a session.
	pub(crate) fn reader(&self) -> InputReader {
		match self {
			InputSource::Process => InputReader::Process(tokio::io::stdin()),
			#[cfg(test)]
			InputSource::Buffer(bytes) => InputReader::Buffer(std::io::Cursor::new(bytes.clone())),
		}
	}
}

/// The reader an [`InputSource`] hands to a session.
pub(crate) enum InputReader {
	/// The process's standard input.
	Process(tokio::io::Stdin),
	/// A fixed buffer.
	#[cfg(test)]
	Buffer(std::io::Cursor<Vec<u8>>),
}

impl AsyncRead for InputReader {
	fn poll_read(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &mut ReadBuf<'_>,
	) -> Poll<std::io::Result<()>> {
		match self.get_mut() {
			InputReader::Process(stdin) => Pin::new(stdin).poll_read(cx, buf),
			#[cfg(test)]
			InputReader::Buffer(cursor) => Pin::new(cursor).poll_read(cx, buf),
		}
	}
}
