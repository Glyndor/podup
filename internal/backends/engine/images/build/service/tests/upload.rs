#![cfg(unix)]

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::Notify;
use tokio::task::{JoinHandle, JoinSet};

use crate::engine::{BuildOptions, Engine};
use crate::libpod::Client;

struct Server(JoinHandle<()>);

impl Drop for Server {
	fn drop(&mut self) {
		self.0.abort();
	}
}

async fn early_response(status: u16, finish_upload: bool) {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("Dockerfile"), "FROM scratch\n").unwrap();
	// Defeat gzip compression and exceed both the bounded tar channel and
	// the socket buffers, so joining the producer before polling fails.
	let mut state = 1u32;
	let data: Vec<u8> = (0..4 * 1024 * 1024)
		.map(|_| {
			state ^= state << 13;
			state ^= state >> 17;
			state ^= state << 5;
			state as u8
		})
		.collect();
	std::fs::write(dir.path().join("payload"), &data).unwrap();
	// Keep the socket outside the context tar.
	let socket_dir = tempfile::tempdir().unwrap();
	let socket = socket_dir.path().join("podman.sock");
	let listener = UnixListener::bind(&socket).unwrap();
	let release = Arc::new(Notify::new());
	let server_release = release.clone();
	let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
	let server = Server(tokio::spawn(async move {
		let mut tasks = JoinSet::new();
		let mut closed_tx = Some(closed_tx);
		loop {
			let (mut io, _) = listener.accept().await.unwrap();
			let release = server_release.clone();
			// Read only the head, without consuming any context bytes.
			let mut head = Vec::new();
			while !head.ends_with(b"\r\n\r\n") {
				let byte = io.read_u8().await.unwrap();
				head.push(byte);
			}
			if !String::from_utf8_lossy(&head).starts_with("POST /v5.0.0/libpod/build?") {
				io.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
					.await
					.unwrap();
				continue;
			}
			let closed_tx = closed_tx.take().unwrap();
			tasks.spawn(async move {
				if finish_upload {
					// A successful head still requires polling DrivenBody while
					// the daemon consumes the request, before its final output.
					io.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
						.await
						.unwrap();
					loop {
						let mut line = Vec::new();
						while !line.ends_with(b"\r\n") {
							line.push(io.read_u8().await.unwrap());
						}
						let size = usize::from_str_radix(
							std::str::from_utf8(&line[..line.len() - 2]).unwrap(),
							16,
						)
						.unwrap();
						let mut chunk = vec![0; size + 2];
						io.read_exact(&mut chunk).await.unwrap();
						if size == 0 {
							break;
						}
					}
					io.write_all(b"0\r\n\r\n").await.unwrap();
				} else {
					let body = b"{\"error\":\"build rejected\"}\n";
					let head = format!(
						"HTTP/1.1 {status} Response\r\nContent-Length: {}\r\n\r\n",
						body.len()
					);
					io.write_all(head.as_bytes()).await.unwrap();
					io.write_all(body).await.unwrap();
					// Do not free upload capacity until the client returns.
					release.notified().await;
					let mut buf = [0; 8192];
					loop {
						match io.read(&mut buf).await {
							Ok(0) | Err(_) => break,
							Ok(_) => {}
						}
					}
				}
				let _ = closed_tx.send(());
			});
		}
	}));
	let engine = Engine::with_base_dir(
		Client::new(socket.to_string_lossy().into_owned()),
		"proj".into(),
		dir.path().to_path_buf(),
	);
	let file = crate::parse_str("services:\n  app:\n    build: .\n").unwrap();
	let result = tokio::time::timeout(
		Duration::from_secs(10),
		engine.build_service(
			"app",
			&file.services["app"],
			&file,
			&BuildOptions {
				quiet: true,
				..Default::default()
			},
		),
	)
	.await
	.expect("early response must not deadlock the upload");
	if finish_upload {
		result.expect("response polling must drive the remaining upload");
	} else {
		let err = result.expect_err("the early daemon error must win");
		assert!(err.to_string().contains("build rejected"));
	}
	release.notify_one();
	tokio::time::timeout(Duration::from_secs(2), closed_rx)
		.await
		.expect("the abandoned upload connection must close")
		.unwrap();
	drop(server);
}

#[tokio::test]
async fn early_in_band_error_cancels_a_backpressured_upload() {
	early_response(200, false).await;
}

#[tokio::test]
async fn early_http_error_cancels_a_backpressured_upload() {
	early_response(400, false).await;
}

#[tokio::test]
async fn early_success_head_drives_upload_and_response_together() {
	early_response(200, true).await;
}
