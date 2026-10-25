//! A fake Podman socket that answers the way a hostile or broken daemon would.
//!
//! Real sockets, real HTTP bytes: the handler gets the raw stream after the
//! request head has been read, so it can write anything, including things no
//! HTTP server would.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;

/// What the client asked for: `GET /v5.0.0/libpod/events?...`.
pub struct Request {
	pub target: String,
}

type Handler = dyn Fn(&mut UnixStream, &Request) + Send + Sync;

/// A listening socket plus the threads that answer on it.
pub struct Fake {
	pub sock: PathBuf,
	stop: Arc<AtomicBool>,
	_dir: TempDir,
}

impl Drop for Fake {
	fn drop(&mut self) {
		self.stop.store(true, Ordering::SeqCst);
	}
}

/// Read the request head. Returns `None` when the peer went away first.
fn read_request(stream: &mut UnixStream) -> Option<Request> {
	let mut head = Vec::new();
	let mut byte = [0u8; 1];
	while !head.ends_with(b"\r\n\r\n") {
		if head.len() > 64 * 1024 || stream.read(&mut byte).ok()? == 0 {
			return None;
		}
		head.push(byte[0]);
	}
	let text = String::from_utf8_lossy(&head);
	let mut first = text.lines().next()?.split_whitespace().skip(1);
	Some(Request {
		target: first.next()?.to_string(),
	})
}

/// Start a fake that calls `handler` once per accepted connection.
pub fn start(handler: impl Fn(&mut UnixStream, &Request) + Send + Sync + 'static) -> Fake {
	let dir = tempfile::tempdir().expect("create socket dir");
	let sock = dir.path().join("podman.sock");
	let listener = UnixListener::bind(&sock).expect("bind fake socket");
	listener.set_nonblocking(true).expect("nonblocking");
	let stop = Arc::new(AtomicBool::new(false));
	let handler: Arc<Handler> = Arc::new(handler);
	let flag = stop.clone();
	std::thread::spawn(move || {
		while !flag.load(Ordering::SeqCst) {
			match listener.accept() {
				Ok((mut stream, _)) => {
					let handler = handler.clone();
					std::thread::spawn(move || {
						stream.set_nonblocking(false).ok();
						if let Some(request) = read_request(&mut stream) {
							handler(&mut stream, &request);
						}
					});
				}
				Err(_) => std::thread::sleep(Duration::from_millis(10)),
			}
		}
	});
	Fake {
		sock,
		stop,
		_dir: dir,
	}
}

/// A complete, honest answer: status, headers, then `body`, then close.
pub fn reply(stream: &mut UnixStream, status: u16, headers: &str, body: &[u8]) {
	let head = format!(
		"HTTP/1.1 {status} X\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n",
		body.len()
	);
	let _ = stream.write_all(head.as_bytes());
	let _ = stream.write_all(body);
}

/// The answer to `_ping` that lets the command carry on.
pub fn ping_ok(stream: &mut UnixStream) {
	reply(stream, 200, "Libpod-API-Version: 5.0.0\r\n", b"OK");
}

/// Write `chunk` until the client hangs up or `limit` bytes have gone out.
/// Returns how many bytes the socket accepted.
pub fn flood(stream: &mut UnixStream, chunk: &[u8], limit: usize) -> usize {
	let mut sent = 0;
	while sent < limit {
		if stream.write_all(chunk).is_err() {
			break;
		}
		sent += chunk.len();
	}
	sent
}

/// Hold the connection open and silent until the client hangs up.
pub fn stall(stream: &mut UnixStream) {
	let mut sink = [0u8; 64];
	stream.set_read_timeout(Some(Duration::from_secs(1))).ok();
	for _ in 0..400 {
		if matches!(stream.read(&mut sink), Ok(0)) {
			return;
		}
	}
}
