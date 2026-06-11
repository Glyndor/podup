//! Multiplexed log/exec stream parser.
//!
//! Docker and Podman use an 8-byte frame header before each payload chunk:
//! `[stream_type: u8][0][0][0][size_big_endian: u32][payload]`
//! Stream type 1 = stdout, 2 = stderr.

use bytes::Bytes;
use futures::Stream;
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

/// Parse a multiplexed stream from a hyper `Incoming` response body.
///
/// Emits [`LogOutput`] items as frames arrive. The returned stream ends when
/// the response body is fully consumed.
pub fn parse_multiplexed(body: Incoming) -> BoxStream<LogOutput> {
	Box::pin(futures::stream::try_unfold(
		(body, Vec::<u8>::new()),
		|(mut body, mut buf)| async move {
			loop {
				// Try to emit a complete frame from the buffer.
				if buf.len() >= 8 {
					let size = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
					if buf.len() >= 8 + size {
						let stream_type = buf[0];
						let payload = Bytes::from(buf[8..8 + size].to_vec());
						buf.drain(..8 + size);
						let output = match stream_type {
							1 => LogOutput::StdOut { message: payload },
							2 => LogOutput::StdErr { message: payload },
							_ => continue, // skip stdin / tty frames
						};
						return Ok(Some((output, (body, buf))));
					}
				}

				// Need more data from the HTTP response body.
				match body.frame().await {
					Some(Ok(frame)) => {
						if let Ok(data) = frame.into_data() {
							buf.extend_from_slice(&data);
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
	Box::pin(futures::stream::try_unfold(
		(body, Vec::<u8>::new()),
		|(mut body, mut buf)| async move {
			loop {
				if let Some(nl) = buf.iter().position(|&b| b == b'\n') {
					let line: Vec<u8> = buf.drain(..nl + 1).take(nl).collect();
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
						}
					}
					Some(Err(e)) => return Err(PodmanError::from(e)),
					None => {
						let line: Vec<u8> = buf.clone();
						buf.clear();
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
