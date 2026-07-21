//! Raw bidirectional streams over the libpod socket, for interactive exec.
//!
//! The rest of the client is connection-per-request through hyper, which is the
//! right shape for a CLI: send, read, done. An interactive exec is not that
//! shape. `POST /exec/{id}/start` with a TTY keeps the connection open in both
//! directions for as long as the command runs — the caller's keystrokes go up
//! while the command's output comes down — so there is no response to read and
//! return.
//!
//! Rather than teach the hyper path to hijack a connection, this writes the
//! request by hand and hands back the socket. The request is trivial (one path,
//! one short JSON body, no redirects, no keep-alive) and hand-writing it keeps
//! the upgrade out of the general client, which every other call would then have
//! to reason about.

use tokio::io::AsyncWriteExt;

use super::{Client, PodmanError, Result};

/// Cap on the response head podup will read before deciding the server is not
/// speaking HTTP. A hijacked stream's head is a status line and a handful of
/// headers; anything larger is a malformed or hostile peer, and reading it
/// unbounded would be a trivial memory exhaustion.
const MAX_HEAD_BYTES: usize = 16 * 1024;

/// A hijacked connection: the socket, after the response head has been read.
///
/// Bytes written go to the command's stdin; bytes read are its output. With a
/// TTY the stream is raw (no 8-byte frame headers) because the pty merges
/// stdout and stderr, which is also why an interactive exec cannot separate
/// them — the same is true of `podman exec -it`.
#[derive(Debug)]
pub(crate) struct Hijacked {
	pub(crate) stream: tokio::net::UnixStream,
}

impl Client {
	/// `POST` a JSON body and keep the connection, for a stream that talks back.
	///
	/// Returns once the response head is read, so a rejected exec (404, 409)
	/// surfaces as an error instead of hanging with the terminal already in raw
	/// mode — which would leave the user's shell unusable.
	pub(crate) async fn post_hijack(&self, path: &str, body: &[u8]) -> Result<Hijacked> {
		let mut stream = tokio::net::UnixStream::connect(&self.socket_path).await?;

		// `Connection: close` is deliberate: this socket is never returned to a
		// pool, and saying so stops the server holding it open after the command
		// exits.
		let head = format!(
			"POST {path} HTTP/1.1\r\n\
			 Host: localhost\r\n\
			 Content-Type: application/json\r\n\
			 Content-Length: {}\r\n\
			 Connection: close\r\n\
			 \r\n",
			body.len()
		);
		stream.write_all(head.as_bytes()).await?;
		stream.write_all(body).await?;
		stream.flush().await?;

		let status = read_response_head(&mut stream).await?;
		if !(200..300).contains(&status) {
			return Err(PodmanError::Api {
				status,
				message: format!("exec start refused with HTTP {status}"),
			});
		}
		Ok(Hijacked { stream })
	}
}

/// Read the response head and return its status code, leaving the socket
/// positioned at the first body byte.
///
/// Reads a byte at a time rather than buffering ahead: a buffered reader would
/// swallow part of the command's output into its own buffer, and that output
/// belongs to the caller. Slow, but the head is a few hundred bytes and it only
/// happens once per exec.
async fn read_response_head(stream: &mut tokio::net::UnixStream) -> Result<u16> {
	use tokio::io::AsyncReadExt;

	let mut head = Vec::with_capacity(256);
	let mut byte = [0u8; 1];
	while !head.ends_with(b"\r\n\r\n") {
		if head.len() >= MAX_HEAD_BYTES {
			return Err(PodmanError::Api {
				status: 0,
				message: "exec start response head exceeded its limit".to_string(),
			});
		}
		let n = stream.read(&mut byte).await?;
		if n == 0 {
			return Err(PodmanError::Api {
				status: 0,
				message: "connection closed before the exec start response".to_string(),
			});
		}
		head.push(byte[0]);
	}

	let text = String::from_utf8_lossy(&head);
	let status_line = text.lines().next().unwrap_or_default();
	// `HTTP/1.1 200 OK` — the code is the second token.
	status_line
		.split_whitespace()
		.nth(1)
		.and_then(|c| c.parse::<u16>().ok())
		.ok_or_else(|| PodmanError::Api {
			status: 0,
			message: format!("unparseable exec start response: {status_line:?}"),
		})
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A refused exec must surface as an error *before* the caller puts the
	/// terminal into raw mode. Returning a live socket for a 404 would hang with
	/// the shell already unusable.
	#[tokio::test]
	async fn a_non_2xx_head_is_an_error_not_a_stream() {
		let dir = tempfile::tempdir().unwrap();
		let sock = dir.path().join("s.sock");
		let listener = tokio::net::UnixListener::bind(&sock).unwrap();
		tokio::spawn(async move {
			if let Ok((mut c, _)) = listener.accept().await {
				use tokio::io::AsyncWriteExt;
				let _ = c
					.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
					.await;
			}
		});

		let client = Client::new(sock.to_string_lossy().to_string());
		let err = client
			.post_hijack("/exec/abc/start", b"{}")
			.await
			.expect_err("a 404 must not yield a stream");
		assert!(err.is_status(404), "got {err:?}");
	}

	/// A server that closes without answering is reported, not treated as a
	/// successful attach.
	#[tokio::test]
	async fn a_closed_connection_is_an_error() {
		let dir = tempfile::tempdir().unwrap();
		let sock = dir.path().join("s.sock");
		let listener = tokio::net::UnixListener::bind(&sock).unwrap();
		tokio::spawn(async move {
			let _ = listener.accept().await;
		});

		let client = Client::new(sock.to_string_lossy().to_string());
		assert!(client.post_hijack("/exec/abc/start", b"{}").await.is_err());
	}
}
