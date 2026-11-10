//! `GitHubSource` over a real socket: the size caps, the retries and the
//! redirects, as the code that calls `read_capped` and `get_with_retry` runs
//! them. The source talks plain http to the loopback here (`loopback`), because
//! no local server can speak TLS to a client that trusts only the public roots.

use super::*;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// What the server does with one connection.
#[derive(Clone)]
enum Reply {
	/// A complete response: status line and headers, then the body.
	Full(String, Vec<u8>),
	/// Headers announcing no length, then bytes until the client goes away.
	Endless,
}

struct Server {
	base: String,
	connections: Arc<AtomicUsize>,
	sent: Arc<AtomicUsize>,
}

fn head(status: &str, extra: &str, length: usize) -> String {
	format!("HTTP/1.1 {status}\r\nContent-Length: {length}\r\nConnection: close\r\n{extra}\r\n")
}

fn ok(body: &[u8]) -> Reply {
	Reply::Full(head("200 OK", "", body.len()), body.to_vec())
}

fn status(code: &str) -> Reply {
	Reply::Full(head(code, "", 0), Vec::new())
}

/// Answers connection `n` with `script[n]`, and the last entry for every one
/// after it.
fn serve(script: Vec<Reply>) -> Server {
	let listener = TcpListener::bind("127.0.0.1:0").unwrap();
	let base = format!("http://{}", listener.local_addr().unwrap());
	let connections = Arc::new(AtomicUsize::new(0));
	let sent = Arc::new(AtomicUsize::new(0));
	let (count, bytes) = (connections.clone(), sent.clone());
	std::thread::spawn(move || {
		for stream in listener.incoming().flatten() {
			let n = count.fetch_add(1, Ordering::SeqCst);
			let reply = script[n.min(script.len() - 1)].clone();
			let bytes = bytes.clone();
			std::thread::spawn(move || answer(stream, reply, &bytes));
		}
	});
	Server {
		base,
		connections,
		sent,
	}
}

fn answer(mut stream: TcpStream, reply: Reply, sent: &AtomicUsize) {
	let mut buf = [0u8; 4096];
	let mut request = Vec::new();
	while !request.windows(4).any(|w| w == b"\r\n\r\n") {
		match stream.read(&mut buf) {
			Ok(0) | Err(_) => return,
			Ok(n) => request.extend_from_slice(&buf[..n]),
		}
	}
	match reply {
		Reply::Full(head, body) => {
			let _ = stream.write_all(head.as_bytes());
			let _ = stream.write_all(&body);
		}
		Reply::Endless => {
			let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n");
			let chunk = [b'x'; 8192];
			while stream.write_all(&chunk).is_ok() {
				sent.fetch_add(chunk.len(), Ordering::SeqCst);
			}
		}
	}
}

impl Server {
	fn connections(&self) -> usize {
		self.connections.load(Ordering::SeqCst)
	}
}

const TAG: &str = r#"{"tag_name":"v1.2.3"}"#;

/// A metadata document of exactly `len` bytes, padded with trailing spaces.
fn metadata_of(len: usize) -> Vec<u8> {
	let mut body = TAG.as_bytes().to_vec();
	body.resize(len, b' ');
	body
}

#[test]
fn the_defaults_are_the_documented_caps_and_backoff() {
	let source = GitHubSource::new(REPO);
	assert_eq!(source.metadata_cap, 1024 * 1024);
	assert_eq!(source.asset_cap, 128 * 1024 * 1024);
	assert_eq!(source.first_delay, std::time::Duration::from_secs(1));
	assert_eq!(ATTEMPTS, 3);
}

#[test]
fn metadata_of_exactly_the_cap_is_read_and_one_byte_more_is_refused() {
	let at = serve(vec![ok(&metadata_of(4096))]);
	let source = GitHubSource::loopback(&at.base, 4096, 0);
	assert_eq!(source.latest_version().unwrap(), "v1.2.3");
	let over = serve(vec![ok(&metadata_of(4097))]);
	let error = GitHubSource::loopback(&over.base, 4096, 0)
		.latest_version()
		.unwrap_err();
	assert!(
		error.to_string().contains("maximum allowed size"),
		"{error}"
	);
}

#[test]
fn an_asset_of_exactly_the_cap_is_read_and_one_byte_more_is_refused() {
	let at = serve(vec![ok(&[7u8; 8192])]);
	let got = GitHubSource::loopback(&at.base, 0, 8192)
		.fetch("podup")
		.unwrap();
	assert_eq!(got.len(), 8192);
	let over = serve(vec![ok(&[7u8; 8193])]);
	let error = GitHubSource::loopback(&over.base, 0, 8192)
		.fetch("podup")
		.unwrap_err();
	assert!(
		error.to_string().contains("maximum allowed size"),
		"{error}"
	);
}

/// A body with no length that never ends: the client has to stop reading at the
/// cap, so the server's writes end shortly after it, not when it runs out of
/// bytes.
#[test]
fn an_endless_download_is_cut_off_near_the_cap() {
	let endless = serve(vec![Reply::Endless]);
	let error = GitHubSource::loopback(&endless.base, 0, 64 * 1024)
		.fetch("podup")
		.unwrap_err();
	assert!(
		error.to_string().contains("maximum allowed size"),
		"{error}"
	);
	// Socket buffers hold some of it; a client that read on would send gigabytes.
	let sent = endless.sent.load(Ordering::SeqCst);
	assert!(sent < 16 * 1024 * 1024, "the server wrote {sent} bytes");
}

#[test]
fn a_client_error_is_asked_once() {
	let server = serve(vec![status("404 Not Found")]);
	let result = GitHubSource::loopback(&server.base, 4096, 4096).latest_version();
	assert!(result.is_err());
	assert_eq!(server.connections(), 1);
}

#[test]
fn a_server_error_is_retried_until_it_answers() {
	let server = serve(vec![
		status("503 Service Unavailable"),
		status("503 Service Unavailable"),
		ok(TAG.as_bytes()),
	]);
	let got = GitHubSource::loopback(&server.base, 4096, 4096).latest_version();
	assert_eq!(got.unwrap(), "v1.2.3");
	assert_eq!(server.connections(), 3);
}

#[test]
fn a_server_that_keeps_failing_is_asked_exactly_the_attempts_allowed() {
	let server = serve(vec![status("503 Service Unavailable")]);
	let result = GitHubSource::loopback(&server.base, 4096, 4096).latest_version();
	assert!(result.is_err());
	assert_eq!(server.connections(), ATTEMPTS as usize);
}

#[test]
fn a_redirect_is_followed_to_the_asset() {
	let location = "Location: /elsewhere\r\n";
	let server = serve(vec![
		Reply::Full(head("302 Found", location, 0), Vec::new()),
		ok(b"asset-bytes"),
	]);
	let got = GitHubSource::loopback(&server.base, 0, 4096).fetch("podup");
	assert_eq!(got.unwrap(), b"asset-bytes");
	assert_eq!(server.connections(), 2);
}

#[test]
fn a_redirect_loop_ends_with_a_bounded_number_of_requests() {
	let location = "Location: /again\r\n";
	let server = serve(vec![Reply::Full(
		head("302 Found", location, 0),
		Vec::new(),
	)]);
	let result = GitHubSource::loopback(&server.base, 0, 4096).fetch("podup");
	assert!(result.is_err());
	// Ten redirects and the first request make eleven, and each of the
	// attempts repeats them.
	let most = ATTEMPTS as usize * 11;
	assert!(
		server.connections() <= most,
		"{} requests",
		server.connections()
	);
}
