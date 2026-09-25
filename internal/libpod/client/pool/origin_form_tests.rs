// The pool's connection-reuse semantics ride on `UnixListener`, which is
// only available on Unix. The pool itself is cross-platform (Windows uses
// a named pipe; see `internal/libpod/client/stream.rs`); the wire
// assertion that pins the request target to origin form is Unix-only by
// necessity because it binds a Unix socket. The `#[cfg(unix)]` here skips
// the test on Windows CI; the pool's other unit tests (which do not bind
// a socket) still run there.
#![cfg(unix)]

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::Mutex;

/// Fake libpod server that captures the request line of every accepted
/// connection and replies with a tiny valid HTTP/1.1 response so the
/// client's pool keeps the socket alive across reads. The response is
/// `Content-Length` (not chunked) because the test cares about what the
/// client wrote on the request side, not how the response frames.
struct CapturingServer {
	sock_path: std::path::PathBuf,
	requests: Arc<Mutex<Vec<String>>>,
	_dir: tempfile::TempDir,
	task: tokio::task::JoinHandle<()>,
}

impl CapturingServer {
	async fn start() -> Self {
		let dir = tempfile::tempdir().unwrap();
		let sock_path = dir.path().join("podman.sock");
		let listener = UnixListener::bind(&sock_path).unwrap();
		let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
		let requests_clone = requests.clone();

		let task = tokio::spawn(async move {
			loop {
				let Ok((mut stream, _)) = listener.accept().await else {
					break;
				};
				let requests_inner = requests_clone.clone();
				tokio::spawn(async move {
					loop {
						let mut buf = Vec::new();
						let mut chunk = [0u8; 1024];
						let mut got_request = false;
						while !got_request {
							match stream.read(&mut chunk).await {
								Ok(0) => return,
								Ok(n) => {
									buf.extend_from_slice(&chunk[..n]);
									if buf.windows(4).any(|w| w == b"\r\n\r\n") {
										got_request = true;
									}
								}
								Err(_) => return,
							}
						}
						let first_line = String::from_utf8_lossy(&buf)
							.lines()
							.next()
							.unwrap_or_default()
							.to_string();
						requests_inner.lock().await.push(first_line);
						let body = b"{}";
						let response = format!(
							"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
							body.len()
						);
						if stream.write_all(response.as_bytes()).await.is_err() {
							return;
						}
						if stream.write_all(body).await.is_err() {
							return;
						}
						if stream.flush().await.is_err() {
							return;
						}
					}
				});
			}
		});

		Self {
			sock_path,
			requests,
			_dir: dir,
			task,
		}
	}

	fn sock_str(&self) -> String {
		self.sock_path.to_string_lossy().into_owned()
	}

	async fn first_request(&self) -> String {
		self.requests
			.lock()
			.await
			.first()
			.cloned()
			.unwrap_or_default()
	}
}

impl Drop for CapturingServer {
	fn drop(&mut self) {
		self.task.abort();
	}
}

/// The wire bytes are what matters. `GET /libpod/_ping` on a real
/// `Client` must reach the socket as `GET /libpod/_ping HTTP/1.1`, not
/// `GET http://localhost/libpod/_ping HTTP/1.1`. Writing the absolute
/// form would put `localhost` (not `libpod`) in
/// `strings.Split(r.URL.String(), "/")[2]`, so Podman would route every
/// shared endpoint to the Docker-compat handlers (#1914). Pin the bytes
/// on the wire because that is the only place the request line lives.
#[tokio::test]
async fn a_get_request_writes_origin_form() {
	let server = CapturingServer::start().await;
	let client = crate::libpod::Client::new(server.sock_str());
	let _: serde_json::Value = client.get_json("/libpod/_ping").await.unwrap();

	let first = server.first_request().await;
	assert!(
		first.starts_with("GET /libpod/_ping HTTP/1.1"),
		"GET must be written in origin form; got: {first:?}"
	);
	assert!(
		!first.contains("http://"),
		"the request target must not contain the absolute-form prefix; got: {first:?}"
	);
}

/// Same check on the streamed-body POST path that `podup build`
/// exercises. Whether the body is framed chunked or content-length, the
/// request line on the wire must still be origin form: the
/// `strings.Split(r.URL.String(), "/")[2]` check happens before hyper
/// looks at the body. Pin the bytes regardless of how the body was
/// framed (#1914).
#[tokio::test]
async fn a_post_with_a_streamed_body_writes_origin_form() {
	use bytes::Bytes;
	use futures_util::stream;
	use hyper::body::Frame;

	let server = CapturingServer::start().await;
	let client = crate::libpod::Client::new(server.sock_str());
	let path = "/v5.0.0/libpod/build?t=probe";
	let chunks = stream::iter(vec![Ok::<_, std::io::Error>(Frame::data(
		Bytes::from_static(b"hello"),
	))]);
	let _resp = client
		.post_stream_body(path, chunks, "application/x-tar")
		.await
		.unwrap();

	let first = server.first_request().await;
	assert!(
		first.starts_with("POST /v5.0.0/libpod/build?t=probe HTTP/1.1"),
		"POST must be written in origin form with the query intact; got: {first:?}"
	);
	assert!(
		!first.contains("http://"),
		"the request target must not contain the absolute-form prefix; got: {first:?}"
	);
}
