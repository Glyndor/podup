//! Multiplexed log/exec stream parser.
//!
//! Docker and Podman use an 8-byte frame header before each payload chunk:
//! `[stream_type: u8][0][0][0][size_big_endian: u32][payload]`
//! Stream type 1 = stdout, 2 = stderr.

use bytes::Bytes;
use futures_util::stream::Stream;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use std::pin::Pin;

use crate::libpod::error::PodmanError;

/// A single framed chunk from a multiplexed container log or exec stream.
#[derive(Debug)]
pub enum LogOutput {
	StdOut { message: Bytes },
	StdErr { message: Bytes },
}

/// Boxed stream alias used for parse_multiplexed and parse_json_lines return types.
pub type BoxStream<T> = Pin<Box<dyn Stream<Item = Result<T, PodmanError>> + Send>>;

/// Upper bound on the reassembly buffer for a single frame or JSON line. Bounds
/// memory when the daemon advertises a huge frame size or never terminates a
/// line, so a rogue or runaway daemon cannot exhaust memory.
const MAX_STREAM_BUF: usize = 256 * 1024 * 1024;

/// Error returned when the reassembly buffer exceeds [`MAX_STREAM_BUF`].
fn stream_buf_overflow() -> PodmanError {
	PodmanError::Api {
		status: 0,
		message: format!("stream chunk exceeds the {MAX_STREAM_BUF} byte limit"),
	}
}

// ---------------------------------------------------------------------------
// Pure parsing helpers (also used by unit tests)
// ---------------------------------------------------------------------------

/// Try to consume one complete multiplexed frame from `buf`.
///
/// Returns `Some((stream_type, payload, bytes_consumed))` if a complete frame
/// is available, or `None` if more data is needed.
pub fn parse_frame(buf: &[u8]) -> Option<(u8, Bytes, usize)> {
	if buf.len() < 8 {
		return None;
	}
	let size = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
	if buf.len() < 8 + size {
		return None;
	}
	let stream_type = buf[0];
	let payload = Bytes::from(buf[8..8 + size].to_vec());
	Some((stream_type, payload, 8 + size))
}

/// Pop the next newline-terminated line from `buf`, excluding the newline byte.
///
/// Returns `Some(line_bytes)` when a `\n` is found, or `None` when no
/// complete line is buffered yet.
pub fn take_json_line(buf: &mut Vec<u8>) -> Option<Vec<u8>> {
	let nl = buf.iter().position(|&b| b == b'\n')?;
	let line: Vec<u8> = buf.drain(..nl + 1).take(nl).collect();
	Some(line)
}

// ---------------------------------------------------------------------------
// Async stream parsers
// ---------------------------------------------------------------------------

/// Parse a multiplexed stream from a hyper `Incoming` response body.
///
/// Emits [`LogOutput`] items as frames arrive. The returned stream ends when
/// the response body is fully consumed.
pub fn parse_multiplexed(body: Incoming) -> BoxStream<LogOutput> {
	Box::pin(futures_util::stream::try_unfold(
		(body, Vec::<u8>::new()),
		|(mut body, mut buf)| async move {
			loop {
				if let Some((stream_type, payload, consumed)) = parse_frame(&buf) {
					buf.drain(..consumed);
					let output = match stream_type {
						1 => LogOutput::StdOut { message: payload },
						2 => LogOutput::StdErr { message: payload },
						_ => continue, // skip stdin / tty frames
					};
					return Ok(Some((output, (body, buf))));
				}

				// Need more data from the HTTP response body.
				match body.frame().await {
					Some(Ok(frame)) => {
						if let Ok(data) = frame.into_data() {
							buf.extend_from_slice(&data);
							if buf.len() > MAX_STREAM_BUF {
								return Err(stream_buf_overflow());
							}
						}
					}
					Some(Err(e)) => return Err(PodmanError::from(e)),
					None => return Ok(None),
				}
			}
		},
	))
}

/// Parse a raw (non-multiplexed) stream from a hyper `Incoming` response body.
///
/// Used for TTY containers where Podman sends raw bytes without 8-byte frame
/// headers. All bytes are treated as stdout since TTY merges the streams.
pub fn parse_raw(body: Incoming) -> BoxStream<LogOutput> {
	Box::pin(futures_util::stream::try_unfold(
		body,
		|mut body| async move {
			loop {
				match body.frame().await {
					Some(Ok(frame)) => {
						if let Ok(data) = frame.into_data() {
							if !data.is_empty() {
								return Ok(Some((LogOutput::StdOut { message: data }, body)));
							}
						}
					}
					Some(Err(e)) => return Err(PodmanError::from(e)),
					None => return Ok(None),
				}
			}
		},
	))
}

/// Parse a newline-delimited JSON stream (used for image pull and build output).
///
/// Each line in the stream is expected to be a complete JSON object. Blank
/// lines between objects are silently skipped.
pub fn parse_json_lines<T: serde::de::DeserializeOwned + Send + 'static>(
	body: Incoming,
) -> BoxStream<T> {
	Box::pin(futures_util::stream::try_unfold(
		(body, Vec::<u8>::new()),
		|(mut body, mut buf)| async move {
			loop {
				if let Some(line) = take_json_line(&mut buf) {
					if line.is_empty() {
						continue;
					}
					let item: T = serde_json::from_slice(&line).map_err(PodmanError::Json)?;
					return Ok(Some((item, (body, buf))));
				}

				match body.frame().await {
					Some(Ok(frame)) => {
						if let Ok(data) = frame.into_data() {
							buf.extend_from_slice(&data);
							if buf.len() > MAX_STREAM_BUF {
								return Err(stream_buf_overflow());
							}
						}
					}
					Some(Err(e)) => return Err(PodmanError::from(e)),
					None => {
						let line = std::mem::take(&mut buf);
						if !line.is_empty() {
							let item: T =
								serde_json::from_slice(&line).map_err(PodmanError::Json)?;
							return Ok(Some((item, (body, buf))));
						}
						return Ok(None);
					}
				}
			}
		},
	))
}

#[cfg(test)]
mod tests {
	use super::*;

	// ---------------------------------------------------------------------------
	// parse_frame tests
	// ---------------------------------------------------------------------------

	#[test]
	fn parse_frame_incomplete_header() {
		assert!(parse_frame(&[0x01, 0x00, 0x00, 0x00]).is_none());
	}

	#[test]
	fn parse_frame_header_present_payload_missing() {
		// Header says 5-byte payload but buffer only has 3.
		let buf = [
			0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, b'a', b'b', b'c',
		];
		assert!(parse_frame(&buf).is_none());
	}

	#[test]
	fn parse_frame_stdout_complete() {
		let payload = b"hello";
		let mut buf = vec![0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05];
		buf.extend_from_slice(payload);
		let (stype, data, consumed) = parse_frame(&buf).unwrap();
		assert_eq!(stype, 1);
		assert_eq!(data.as_ref(), b"hello");
		assert_eq!(consumed, 13);
	}

	#[test]
	fn parse_frame_stderr_complete() {
		let payload = b"err";
		let mut buf = vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03];
		buf.extend_from_slice(payload);
		let (stype, data, consumed) = parse_frame(&buf).unwrap();
		assert_eq!(stype, 2);
		assert_eq!(data.as_ref(), b"err");
		assert_eq!(consumed, 11);
	}

	#[test]
	fn parse_frame_zero_length_payload() {
		let buf = [0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
		let (stype, data, consumed) = parse_frame(&buf).unwrap();
		assert_eq!(stype, 1);
		assert!(data.is_empty());
		assert_eq!(consumed, 8);
	}

	#[test]
	fn parse_frame_leaves_remainder() {
		// Buffer has one full frame + extra bytes.
		let mut buf = vec![0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, b'h', b'i'];
		buf.extend_from_slice(b"leftover");
		let (_, data, consumed) = parse_frame(&buf).unwrap();
		assert_eq!(data.as_ref(), b"hi");
		assert_eq!(consumed, 10);
		assert_eq!(&buf[consumed..], b"leftover");
	}

	// ---------------------------------------------------------------------------
	// take_json_line tests
	// ---------------------------------------------------------------------------

	#[test]
	fn take_json_line_no_newline() {
		let mut buf = b"partial line".to_vec();
		assert!(take_json_line(&mut buf).is_none());
		assert_eq!(buf, b"partial line");
	}

	#[test]
	fn take_json_line_with_newline() {
		let mut buf = b"line1\nline2".to_vec();
		let line = take_json_line(&mut buf).unwrap();
		assert_eq!(line, b"line1");
		assert_eq!(buf, b"line2");
	}

	#[test]
	fn take_json_line_empty_line() {
		let mut buf = b"\nnext".to_vec();
		let line = take_json_line(&mut buf).unwrap();
		assert!(line.is_empty());
		assert_eq!(buf, b"next");
	}

	#[test]
	fn take_json_line_multiple_lines() {
		let mut buf = b"a\nb\nc".to_vec();
		assert_eq!(take_json_line(&mut buf).unwrap(), b"a");
		assert_eq!(take_json_line(&mut buf).unwrap(), b"b");
		assert!(take_json_line(&mut buf).is_none());
	}
}
