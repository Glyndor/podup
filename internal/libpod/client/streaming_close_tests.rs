//! Streaming path regression for the `Connection: close` short-body case.
//!
//! The end-to-end wire contract for a short response the server
//! terminates with `Connection: close`: the head and the whole body come
//! back through the streaming path on a single socket. The
//! `Future`-contract side (the connection future is not re-polled after
//! the driver finishes in the same poll as the head) is pinned by the
//! `conn_state_*` unit tests in `conn_state_tests.rs`; this test pins
//! the end-to-end behaviour against a real socket (#1900).

#![cfg(unix)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use http_body_util::BodyExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

use super::Client;

/// A fake libpod server that answers one request with `Content-Length` and
/// `Connection: close`, writes the body, and then closes the socket so the
/// driver future finishes inside the same poll that delivers the head.
struct CloseServer {
	sock_path: std::path::PathBuf,
	accepted: Arc<AtomicUsize>,
	_dir: tempfile::TempDir,
	task: tokio::task::JoinHandle<()>,
}

impl CloseServer {
	async fn start(body: Vec<u8>) -> Self {
		let dir = tempfile::tempdir().unwrap();
		let sock_path = dir.path().join("podman.sock");
		let listener = UnixListener::bind(&sock_path).unwrap();
		let accepted = Arc::new(AtomicUsize::new(0));
		let accepted_clone = accepted.clone();

		let task = tokio::spawn(async move {
			loop {
				let Ok((mut stream, _)) = listener.accept().await else {
					break;
				};
				accepted_clone.fetch_add(1, Ordering::SeqCst);
				// Serve exactly one request, then close. The next call to
				// `Client::get_stream` would race the closed socket; the
				// test only needs the single round-trip, so the loop
				// exits when the stream ends.
				let body_len = body.len();
				let mut buf = Vec::new();
				let mut chunk = [0u8; 1024];
				let mut got_request = false;
				while !got_request {
					match stream.read(&mut chunk).await {
						Ok(0) | Err(_) => return,
						Ok(n) => {
							buf.extend_from_slice(&chunk[..n]);
							if buf.windows(4).any(|w| w == b"\r\n\r\n") {
								got_request = true;
							}
						}
					}
				}
				let headers = format!(
					"HTTP/1.1 200 OK\r\n\
					 content-type: application/octet-stream\r\n\
					 content-length: {body_len}\r\n\
					 connection: close\r\n\
					 \r\n"
				);
				if stream.write_all(headers.as_bytes()).await.is_err() {
					return;
				}
				if stream.write_all(&body).await.is_err() {
					return;
				}
				if stream.flush().await.is_err() {
					return;
				}
				let _ = stream.shutdown().await;
			}
		});

		Self {
			sock_path,
			accepted,
			_dir: dir,
			task,
		}
	}

	fn sock_str(&self) -> String {
		self.sock_path.to_string_lossy().into_owned()
	}
}

impl Drop for CloseServer {
	fn drop(&mut self) {
		self.task.abort();
	}
}

/// A short response sent with `Content-Length` and `Connection: close`
/// comes back whole through the streaming path (`get_stream`), over a
/// single socket. The whole body fits in one server-side write, so a
/// single `accept` covers head and body; the test asserts the bytes the
/// daemon promised are exactly the bytes the caller sees, and that no
/// second socket had to be opened to deliver them (#1900).
///
/// The `Future`-contract side of the streaming path (the connection
/// future is not re-polled after the driver finishes) is pinned by the
/// `conn_state_*` unit tests in `conn_state_tests.rs`; this test stays
/// focused on the end-to-end wire behaviour.
#[tokio::test]
async fn streaming_path_serves_connection_close_short_body_cleanly() {
	let body = b"the quick brown fox jumps over the lazy dog".repeat(8);
	let server = CloseServer::start(body.clone()).await;
	let client = Client::with_pool_size(server.sock_str(), 1);

	let resp = client
		.get_stream("/libpod/_ping")
		.await
		.expect("streaming short body with Connection: close must not fail");
	let collected = resp
		.into_body()
		.collect()
		.await
		.expect("streaming short body must drain to completion");
	let bytes = collected.to_bytes();
	assert_eq!(
		bytes.as_ref(),
		body.as_slice(),
		"the streaming path returned the full short body byte-for-byte"
	);
	assert_eq!(
		server.accepted.load(Ordering::SeqCst),
		1,
		"exactly one socket opened (the body came back over the same connection)"
	);
}
