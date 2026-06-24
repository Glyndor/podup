//! Multiplexed log/exec stream parser.
//!
//! Docker and Podman use an 8-byte frame header before each payload chunk:
//! `[stream_type: u8][0][0][0][size_big_endian: u32][payload]`
//! Stream type 1 = stdout, 2 = stderr.

use bytes::{Bytes, BytesMut};
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

/// Try to consume one complete multiplexed frame from the front of `buf`.
///
/// On success the 8-byte header and its payload are split off the front of
/// `buf` (the remaining bytes stay buffered for the next frame) and
/// `Some((stream_type, payload))` is returned. The payload is a zero-copy
/// [`Bytes`] sharing the original allocation, so no per-frame copy or tail
/// memmove occurs. Returns `None` (leaving `buf` untouched) when fewer than a
/// full frame is buffered and more data is needed.
pub fn parse_frame(buf: &mut BytesMut) -> Option<(u8, Bytes)> {
	if buf.len() < 8 {
		return None;
	}
	let size = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
	if buf.len() < 8 + size {
		return None;
	}
	let stream_type = buf[0];
	// Split the header + payload off the front of `buf` in O(1); the leftover
	// bytes remain in `buf` without being moved.
	let mut frame = buf.split_to(8 + size);
	let payload = frame.split_off(8).freeze();
	Some((stream_type, payload))
}

/// Pop the next newline-terminated line from the front of `buf`, excluding the
/// newline byte.
///
/// On success the line and its trailing `\n` are split off the front of `buf`
/// in O(1) (no tail memmove) and the line is returned as a zero-copy [`Bytes`]
/// sharing the original allocation. Returns `None` (leaving `buf` untouched)
/// when no complete line is buffered yet.
pub fn take_json_line(buf: &mut BytesMut) -> Option<Bytes> {
	let nl = buf.iter().position(|&b| b == b'\n')?;
	let mut line = buf.split_to(nl + 1);
	line.truncate(nl); // drop the trailing newline byte
	Some(line.freeze())
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
		(body, BytesMut::new()),
		|(mut body, mut buf)| async move {
			loop {
				if let Some((stream_type, payload)) = parse_frame(&mut buf) {
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
		(body, BytesMut::new()),
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
						// Trailing bytes with no terminating newline: parse the
						// remainder as a final line.
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
		let mut buf = BytesMut::from(&[0x01, 0x00, 0x00, 0x00][..]);
		assert!(parse_frame(&mut buf).is_none());
		// A `None` result must leave the buffer untouched.
		assert_eq!(buf.as_ref(), &[0x01, 0x00, 0x00, 0x00]);
	}

	#[test]
	fn parse_frame_header_present_payload_missing() {
		// Header says 5-byte payload but buffer only has 3.
		let mut buf = BytesMut::from(
			&[
				0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, b'a', b'b', b'c',
			][..],
		);
		assert!(parse_frame(&mut buf).is_none());
		// Partial frame stays buffered for the next read.
		assert_eq!(buf.len(), 11);
	}

	#[test]
	fn parse_frame_stdout_complete() {
		let mut buf = BytesMut::from(&[0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05][..]);
		buf.extend_from_slice(b"hello");
		let (stype, data) = parse_frame(&mut buf).unwrap();
		assert_eq!(stype, 1);
		assert_eq!(data.as_ref(), b"hello");
		// The full frame is consumed from the front.
		assert!(buf.is_empty());
	}

	#[test]
	fn parse_frame_stderr_complete() {
		let mut buf = BytesMut::from(&[0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03][..]);
		buf.extend_from_slice(b"err");
		let (stype, data) = parse_frame(&mut buf).unwrap();
		assert_eq!(stype, 2);
		assert_eq!(data.as_ref(), b"err");
		assert!(buf.is_empty());
	}

	#[test]
	fn parse_frame_zero_length_payload() {
		let mut buf = BytesMut::from(&[0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00][..]);
		let (stype, data) = parse_frame(&mut buf).unwrap();
		assert_eq!(stype, 1);
		assert!(data.is_empty());
		assert!(buf.is_empty());
	}

	#[test]
	fn parse_frame_leaves_remainder() {
		// Buffer has one full frame + extra bytes.
		let mut buf =
			BytesMut::from(&[0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, b'h', b'i'][..]);
		buf.extend_from_slice(b"leftover");
		let (_, data) = parse_frame(&mut buf).unwrap();
		assert_eq!(data.as_ref(), b"hi");
		// Only the consumed frame is removed; the remainder is left in place.
		assert_eq!(buf.as_ref(), b"leftover");
	}

	#[test]
	fn parse_frame_two_frames_in_one_buffer_demux() {
		// One stdout frame ("hi") immediately followed by one stderr frame
		// ("er") must demux to the correct stream, in order.
		let mut buf =
			BytesMut::from(&[0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, b'h', b'i'][..]);
		buf.extend_from_slice(&[0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, b'e', b'r']);
		let (stype1, data1) = parse_frame(&mut buf).unwrap();
		assert_eq!(stype1, 1);
		assert_eq!(data1.as_ref(), b"hi");
		let (stype2, data2) = parse_frame(&mut buf).unwrap();
		assert_eq!(stype2, 2);
		assert_eq!(data2.as_ref(), b"er");
		assert!(buf.is_empty());
		assert!(parse_frame(&mut buf).is_none());
	}

	#[test]
	fn parse_frame_split_across_reads_reassembles() {
		// First read delivers only the header plus a partial payload; the frame
		// must not parse until the rest arrives in a second read.
		let mut buf = BytesMut::from(&[0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05][..]);
		buf.extend_from_slice(b"hel");
		assert!(parse_frame(&mut buf).is_none());
		// Second read completes the payload.
		buf.extend_from_slice(b"lo");
		let (stype, data) = parse_frame(&mut buf).unwrap();
		assert_eq!(stype, 1);
		assert_eq!(data.as_ref(), b"hello");
		assert!(buf.is_empty());
	}

	// ---------------------------------------------------------------------------
	// take_json_line tests
	// ---------------------------------------------------------------------------

	#[test]
	fn take_json_line_no_newline() {
		let mut buf = BytesMut::from(&b"partial line"[..]);
		assert!(take_json_line(&mut buf).is_none());
		assert_eq!(buf.as_ref(), b"partial line");
	}

	#[test]
	fn take_json_line_with_newline() {
		let mut buf = BytesMut::from(&b"line1\nline2"[..]);
		let line = take_json_line(&mut buf).unwrap();
		assert_eq!(line.as_ref(), b"line1");
		assert_eq!(buf.as_ref(), b"line2");
	}

	#[test]
	fn take_json_line_empty_line() {
		let mut buf = BytesMut::from(&b"\nnext"[..]);
		let line = take_json_line(&mut buf).unwrap();
		assert!(line.is_empty());
		assert_eq!(buf.as_ref(), b"next");
	}

	#[test]
	fn take_json_line_multiple_lines() {
		let mut buf = BytesMut::from(&b"a\nb\nc"[..]);
		assert_eq!(take_json_line(&mut buf).unwrap().as_ref(), b"a");
		assert_eq!(take_json_line(&mut buf).unwrap().as_ref(), b"b");
		assert!(take_json_line(&mut buf).is_none());
	}

	#[test]
	fn take_json_line_multiple_lines_in_one_buffer_in_order() {
		// Several complete JSON lines delivered in a single buffer fill must be
		// returned one at a time, in arrival order, with the remainder kept.
		let mut buf = BytesMut::from(
			&br#"{"a":1}
{"b":2}
{"c":3}
"#[..],
		);
		assert_eq!(take_json_line(&mut buf).unwrap().as_ref(), br#"{"a":1}"#);
		assert_eq!(take_json_line(&mut buf).unwrap().as_ref(), br#"{"b":2}"#);
		assert_eq!(take_json_line(&mut buf).unwrap().as_ref(), br#"{"c":3}"#);
		assert!(take_json_line(&mut buf).is_none());
		assert!(buf.is_empty());
	}

	#[test]
	fn take_json_line_split_across_reads_reassembles() {
		// A line whose newline only arrives in the second read must not be
		// returned until that read completes it.
		let mut buf = BytesMut::from(&b"{\"a\":"[..]);
		assert!(take_json_line(&mut buf).is_none());
		buf.extend_from_slice(b"1}\n");
		assert_eq!(take_json_line(&mut buf).unwrap().as_ref(), br#"{"a":1}"#);
		assert!(buf.is_empty());
	}

	// ---------------------------------------------------------------------------
	// MAX_STREAM_BUF cap
	// ---------------------------------------------------------------------------

	/// Mirror of the cap check the async parsers run after each buffer fill
	/// (`buf.len() > MAX_STREAM_BUF`). Returns the overflow error when, and only
	/// when, the reassembly buffer has grown strictly past the limit.
	fn cap_check(buf_len: usize) -> Option<PodmanError> {
		if buf_len > MAX_STREAM_BUF {
			Some(stream_buf_overflow())
		} else {
			None
		}
	}

	#[test]
	fn over_cap_buffer_is_rejected() {
		// A buffer that grows one byte past the cap must trip the overflow guard
		// with the documented Api error (status 0, message naming the limit).
		match cap_check(MAX_STREAM_BUF + 1) {
			Some(PodmanError::Api { status, message }) => {
				assert_eq!(status, 0);
				assert!(message.contains(&MAX_STREAM_BUF.to_string()));
			}
			other => panic!("expected Api overflow error, got {other:?}"),
		}
	}

	#[test]
	fn at_cap_buffer_is_accepted() {
		// A buffer exactly at the cap must be allowed; the guard rejects only a
		// buffer strictly greater than MAX_STREAM_BUF.
		assert!(cap_check(MAX_STREAM_BUF).is_none());
	}
}
