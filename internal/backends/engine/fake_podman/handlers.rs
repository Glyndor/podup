//! The wire side of the fake Podman socket: reading one request and writing
//! the canned answer back.
//!
//! It is the half of the module root that reads the request and frames the
//! response; the root keeps the shape of the reply and the socket itself.
//! Split into its own file because the root had grown past the size the
//! standard allows, and the behaviour is the one it always had.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use super::{FakeReply, Responder};

/// Read one HTTP/1.1 request line (every request this harness serves carries
/// an empty body, so the header block is the whole request) and write back
/// the canned response.
pub(super) async fn serve_one(
	mut stream: UnixStream,
	respond: &Responder,
	requests: &Mutex<Vec<String>>,
	bodies: &Mutex<Vec<Vec<u8>>>,
	content_types: &Mutex<Vec<String>>,
) -> std::io::Result<()> {
	let mut buf = Vec::new();
	let mut chunk = [0u8; 1024];
	loop {
		let n = stream.read(&mut chunk).await?;
		if n == 0 {
			break;
		}
		buf.extend_from_slice(&chunk[..n]);
		if buf.windows(4).any(|w| w == b"\r\n\r\n") {
			break;
		}
	}

	let head = String::from_utf8_lossy(&buf);
	let request_line = head.lines().next().unwrap_or_default();
	let mut parts = request_line.split_whitespace();
	let method = parts.next().unwrap_or_default().to_string();
	let target = parts.next().unwrap_or_default().to_string();

	// Capture the body so tests can assert on the JSON payload of a POST
	// (e.g. the `SpecGenerator` sent to `/containers/create`). hyper reports
	// the body size up front as `Content-Length`, so reading exactly that many
	// bytes after the header terminator reaches the end of the request. A
	// missing or non-numeric header leaves the body empty, which is what every
	// pre-existing test relied on.
	//
	// When the client does not know the body size up front, `post_stream_body`
	// (`POST /libpod/build` and the build-context tar), hyper falls back to
	// `Transfer-Encoding: chunked` and the wire shape is a sequence of
	// `<hex-len>\r\n<bytes>\r\n` blocks ending in `0\r\n\r\n`. The pre-existing
	// tests never exercised that path; `build` (#1681) does, and needs the
	// fake to read it so the body can complete and the response head can fly.
	let content_length: Option<usize> = head
		.lines()
		.find_map(|l| {
			l.strip_prefix("Content-Length: ")
				.or_else(|| l.strip_prefix("content-length: "))
		})
		.and_then(|v| v.trim().parse::<usize>().ok());
	let chunked = head
		.lines()
		.any(|l| l.eq_ignore_ascii_case("transfer-encoding: chunked"));
	let head_end = head.find("\r\n\r\n").map(|i| i + 4).unwrap_or(buf.len());
	let body = if chunked {
		let mut body = Vec::new();
		read_chunked_body(&mut stream, &buf[head_end..], &mut body).await?;
		body
	} else {
		match content_length {
			Some(n) if n > 0 => {
				// Drain the bytes already read past the header terminator.
				let already = buf.len() - head_end;
				let mut body = Vec::with_capacity(n);
				if already >= n {
					body.extend_from_slice(&buf[head_end..head_end + n]);
				} else {
					body.extend_from_slice(&buf[head_end..]);
					let mut remaining = n - body.len();
					let mut tail = [0u8; 1024];
					while remaining > 0 {
						let take = remaining.min(tail.len());
						let n = stream.read(&mut tail[..take]).await?;
						if n == 0 {
							break;
						}
						body.extend_from_slice(&tail[..n]);
						remaining -= n;
					}
				}
				body
			}
			_ => Vec::new(),
		}
	};

	requests.lock().unwrap().push(format!("{method} {target}"));
	bodies.lock().unwrap().push(body);
	content_types.lock().unwrap().push(
		head.lines()
			.find_map(|l| {
				l.strip_prefix("Content-Type: ")
					.or_else(|| l.strip_prefix("content-type: "))
			})
			.unwrap_or_default()
			.trim()
			.to_string(),
	);

	match respond(&method, &target) {
		FakeReply::Body(status, body) => {
			let response = format!(
				"HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {len}\r\nconnection: close\r\n\r\n{body}",
				reason = reason_phrase(status),
				len = body.len(),
			);
			stream.write_all(response.as_bytes()).await?;
		}
		FakeReply::ChunkedEnd(chunks) => {
			write_chunked(&mut stream, &chunks).await?;
			// The terminating zero-length chunk: this is the difference between
			// a finished body and a severed one, and it is the whole subject of
			// #1104.
			stream.write_all(b"0\r\n\r\n").await?;
		}
		FakeReply::ChunkedTruncated(chunks) => {
			write_chunked(&mut stream, &chunks).await?;
			// No terminator. Closing here is what a dead stream looks like.
		}
		FakeReply::ChunkedCutMidPayload(chunk) => {
			write_chunked(&mut stream, &[]).await?;
			// Promise the whole chunk, deliver half of it, hang up.
			let half = chunk.len() / 2;
			stream
				.write_all(format!("{:x}\r\n{}", chunk.len(), &chunk[..half]).as_bytes())
				.await?;
			stream.flush().await?;
		}
		FakeReply::ClosedWithoutResponse => {
			// Write nothing. The shutdown below is the entire reply.
		}
		FakeReply::Silent => {
			// The request was read; the connection stays open without a
			// response, so the client blocks on the read until its own
			// deadline ends the wait. The hold is bounded so a client that
			// never times out cannot pin the fake.
			tokio::time::sleep(SILENT_HOLD).await;
		}
		FakeReply::Headers(status, headers) => {
			let mut response = format!(
				"HTTP/1.1 {status} {reason}\r\ncontent-length: 0\r\nconnection: close\r\n",
				reason = reason_phrase(status),
			);
			for (name, value) in headers {
				response.push_str(&format!("{name}: {value}\r\n"));
			}
			response.push_str("\r\n");
			stream.write_all(response.as_bytes()).await?;
		}
		FakeReply::Hijacked {
			head,
			frames,
			received,
		} => {
			stream.write_all(head.as_bytes()).await?;
			for frame in frames {
				stream.write_all(&frame).await?;
			}
			stream.flush().await?;
			// The client may still be sending: an exec session's stdin travels
			// on this connection, and the client closes its half when its own
			// input ends. Read what arrives until it does, bounded so a client
			// that never closes cannot pin the fake.
			let _ = tokio::time::timeout(
				HIJACK_READ_BUDGET,
				read_client_bytes(&mut stream, &received),
			)
			.await;
		}
	}
	stream.shutdown().await?;
	Ok(())
}

/// How long the fake waits for the client to finish sending on a hijacked
/// connection before giving up on it.
const HIJACK_READ_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);

/// How long the fake holds a silent connection open before letting it go.
///
/// The hold is longer than any client deadline the engine uses, so the client
/// is the one that gives up; the fake only needs to keep the peer from
/// answering for that long.
const SILENT_HOLD: std::time::Duration = std::time::Duration::from_secs(600);

/// Read what the client sends on a hijacked connection into `received`, until
/// the client closes its half of it.
async fn read_client_bytes(
	stream: &mut UnixStream,
	received: &Arc<Mutex<Vec<u8>>>,
) -> std::io::Result<()> {
	let mut buf = [0u8; 4096];
	loop {
		let n = stream.read(&mut buf).await?;
		if n == 0 {
			return Ok(());
		}
		received.lock().unwrap().extend_from_slice(&buf[..n]);
	}
}

/// Reason phrase for the statuses this harness's tests use; anything else
/// falls back to a placeholder (the client only parses the numeric code).
fn reason_phrase(status: u16) -> &'static str {
	match status {
		200 => "OK",
		204 => "No Content",
		304 => "Not Modified",
		404 => "Not Found",
		409 => "Conflict",
		500 => "Internal Server Error",
		_ => "Unknown",
	}
}

/// Write a chunked-encoding response head and one chunk per entry, leaving the
/// caller to decide whether the terminating chunk follows.
async fn write_chunked(stream: &mut UnixStream, chunks: &[String]) -> std::io::Result<()> {
	stream
		.write_all(
			b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n",
		)
		.await?;
	for chunk in chunks {
		stream
			.write_all(format!("{:x}\r\n{chunk}\r\n", chunk.len()).as_bytes())
			.await?;
	}
	stream.flush().await
}

/// Read a `Transfer-Encoding: chunked` request body into `out`. `prefix` is
/// whatever was already in the read buffer past the `\r\n\r\n` header
/// terminator; the rest comes off the socket. Walks the chunked wire shape
/// until the zero-length terminator, appending each chunk's payload bytes to
/// `out` and ignoring the chunk-extension / trailer framing the client is
/// allowed to send.
async fn read_chunked_body(
	stream: &mut UnixStream,
	prefix: &[u8],
	out: &mut Vec<u8>,
) -> std::io::Result<()> {
	let mut buf: Vec<u8> = prefix.to_vec();
	loop {
		// Pull enough bytes to find the next `\r\n`, which terminates a chunk
		// size header. Hyper's StreamBody emits short chunks (one frame per
		// mpsc receive), so a 4 KiB read window is plenty.
		while !buf.windows(2).any(|w| w == b"\r\n") {
			let mut tmp = [0u8; 4096];
			let read = stream.read(&mut tmp).await?;
			if read == 0 {
				return Ok(());
			}
			buf.extend_from_slice(&tmp[..read]);
		}
		let nl = buf.windows(2).position(|w| w == b"\r\n").unwrap();
		let size_line = std::str::from_utf8(&buf[..nl])
			.map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
		// Chunk extensions (`name=value;name=value`) are legal; split off the
		// size token at the first `;`. We do not act on any extension.
		let size_tok = size_line.split(';').next().unwrap_or("").trim();
		let size: usize = match usize::from_str_radix(size_tok, 16) {
			Ok(n) => n,
			Err(_) => {
				return Err(std::io::Error::new(
					std::io::ErrorKind::InvalidData,
					format!("invalid chunk size '{size_tok}'"),
				));
			}
		};
		// Drop the size line; what remains starts at the chunk's payload (or
		// at the next chunk's size header if `size == 0`).
		buf.drain(..nl + 2);
		if size == 0 {
			// Trailing `\r\n` is optional in HTTP/1.1; consume what the
			// client sent (one or two bytes) and stop.
			while buf.len() < 2 {
				let mut tmp = [0u8; 2];
				let read = stream.read(&mut tmp).await?;
				if read == 0 {
					break;
				}
				buf.extend_from_slice(&tmp[..read]);
			}
			buf.drain(..buf.len().min(2));
			return Ok(());
		}
		// Make sure we have the full chunk (and its trailing CRLF) in `buf`,
		// pulling more bytes if needed.
		while buf.len() < size + 2 {
			let mut tmp = [0u8; 4096];
			let read = stream.read(&mut tmp).await?;
			if read == 0 {
				return Ok(());
			}
			buf.extend_from_slice(&tmp[..read]);
		}
		out.extend_from_slice(&buf[..size]);
		buf.drain(..size + 2);
	}
}
