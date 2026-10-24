//! In-memory HTTP peer for init lifecycle coverage without a daemon or socket.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

use super::Client;

type Reply = dyn Fn(&str, &str, &serde_json::Value) -> (u16, String) + Send + Sync;

thread_local! {
	static PEER: RefCell<Option<Arc<Reply>>> = const { RefCell::new(None) };
}

/// Keep the HTTP requests available to lifecycle assertions.
pub(crate) struct Peer {
	/// Preserve request order and container specifications for assertions.
	pub(crate) requests: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
}

impl Peer {
	/// Select this peer through the transport without opening a socket.
	pub(crate) fn client(&self) -> Client {
		Client::new("pre-start-memory")
	}
}

impl Drop for Peer {
	fn drop(&mut self) {
		PEER.with(|peer| peer.replace(None));
	}
}

/// Install a peer only on the current test runtime's thread.
pub(crate) fn start(
	reply: impl Fn(&str, &str, &serde_json::Value) -> (u16, String) + Send + Sync + 'static,
) -> Peer {
	let requests = Arc::new(Mutex::new(Vec::new()));
	let seen = requests.clone();
	PEER.with(|peer| {
		assert!(peer.borrow().is_none());
		peer.replace(Some(Arc::new(move |method, path, body| {
			seen.lock()
				.unwrap()
				.push((format!("{method} {path}"), body.clone()));
			reply(method, path, body)
		})));
	});
	Peer { requests }
}

/// Route only the explicit in-memory path, leaving real transports untouched.
pub(crate) fn connect(path: &str) -> Option<DuplexStream> {
	if path != "pre-start-memory" {
		return None;
	}
	let reply = PEER.with(|peer| peer.borrow().as_ref().unwrap().clone());
	let (client, mut server) = tokio::io::duplex(65536);
	tokio::spawn(async move {
		let mut bytes = Vec::new();
		let mut chunk = [0; 4096];
		let head_end = loop {
			let n = server.read(&mut chunk).await.unwrap();
			assert_ne!(n, 0);
			bytes.extend_from_slice(&chunk[..n]);
			if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
				break end + 4;
			}
		};
		let head = String::from_utf8(bytes[..head_end].to_vec()).unwrap();
		let length: usize = head
			.lines()
			.filter_map(|line| line.split_once(':'))
			.find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
			.map_or(0, |(_, value)| value.trim().parse().unwrap());
		while bytes.len() < head_end + length {
			let n = server.read(&mut chunk).await.unwrap();
			assert_ne!(n, 0);
			bytes.extend_from_slice(&chunk[..n]);
		}
		let mut first = head.lines().next().unwrap().split_whitespace();
		let body = serde_json::from_slice(&bytes[head_end..]).unwrap_or_default();
		let (status, body) = reply(first.next().unwrap(), first.next().unwrap(), &body);
		let response = format!(
			"HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
			body.len()
		);
		let _ = server.write_all(response.as_bytes()).await;
	});
	Some(client)
}
