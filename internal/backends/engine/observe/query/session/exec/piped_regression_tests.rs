//! Backpressure and completion of a hijacked non-TTY exec session.
#![cfg(unix)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::pump_piped;
use crate::engine::fake_podman::{self, FakePodman, FakeReply};

fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
	let mut bytes = vec![kind, 0, 0, 0];
	bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
	bytes.extend_from_slice(payload);
	bytes
}

fn fake_session(frames: Vec<Vec<u8>>) -> (FakePodman, Arc<Mutex<Vec<u8>>>) {
	let received = Arc::new(Mutex::new(Vec::new()));
	let record = received.clone();
	let fake = fake_podman::start_replying(move |_, _| FakeReply::Hijacked {
		head: "HTTP/1.1 200 OK\r\nContent-Type: application/vnd.docker.multiplexed-stream\r\n\r\n"
			.into(),
		frames: frames.clone(),
		received: record.clone(),
	});
	(fake, received)
}

async fn hijack(fake: &FakePodman) -> crate::libpod::client::Hijacked {
	fake.client()
		.post_hijack(
			"/v5.0.0/libpod/exec/test/start",
			br#"{"Detach":false,"Tty":false}"#,
		)
		.await
		.unwrap()
}

/// Gate the peer's input consumption on output delivery, with a buffer small
/// enough that a single stdin write cannot finish. No sleeps or socket-buffer
/// size assumptions are needed to reproduce the mutual wait.
#[tokio::test]
async fn blocked_stdin_writes_do_not_stop_output_reads() {
	let payload = vec![b'o'; 128];
	let wire = frame(1, &payload);
	let wire_len = wire.len();
	let (fake, received) = fake_session(vec![wire]);
	let hijacked = hijack(&fake).await;
	let (proxy, session) = tokio::io::duplex(64);
	let bridge = tokio::spawn(async move {
		let (mut peer_read, mut peer_write) = tokio::io::split(hijacked.stream);
		let (mut input_read, mut output_write) = tokio::io::split(proxy);
		let (input_started, start_output) = tokio::sync::oneshot::channel();
		let (output_sent, resume_input) = tokio::sync::oneshot::channel();
		let input = async {
			let mut first = [0u8; 1];
			input_read.read_exact(&mut first).await.unwrap();
			input_started.send(()).unwrap();
			resume_input.await.unwrap();
			peer_write.write_all(&first).await.unwrap();
			tokio::io::copy(&mut input_read, &mut peer_write)
				.await
				.unwrap();
			peer_write.shutdown().await.unwrap();
		};
		let output = async {
			start_output.await.unwrap();
			let mut bytes = vec![0; wire_len];
			peer_read.read_exact(&mut bytes).await.unwrap();
			output_write.write_all(&bytes).await.unwrap();
			output_sent.send(()).unwrap();
			tokio::io::copy(&mut peer_read, &mut output_write)
				.await
				.unwrap();
			output_write.shutdown().await.unwrap();
		};
		tokio::join!(input, output);
	});
	let input = vec![b'i'; 8192];
	let mut stdout = Vec::new();
	let mut stderr = Vec::new();
	let result = tokio::time::timeout(
		Duration::from_secs(2),
		pump_piped(
			session,
			std::io::Cursor::new(input.clone()),
			&mut stdout,
			&mut stderr,
		),
	)
	.await;
	if result.is_err() {
		bridge.abort();
	}
	result
		.expect("both directions must progress independently")
		.unwrap();
	bridge.await.unwrap();
	assert_eq!(stdout, payload);
	assert!(stderr.is_empty());
	assert_eq!(*received.lock().unwrap(), input);
}

#[tokio::test]
async fn blocked_output_writes_do_not_stop_stdin_reads() {
	let payload = vec![b'o'; 128];
	let (fake, received) = fake_session(vec![frame(1, &payload)]);
	let hijacked = hijack(&fake).await;
	let (mut input_write, input_read) = tokio::io::duplex(64);
	let (output_write, mut output_read) = tokio::io::duplex(64);
	let peer = tokio::spawn(async move {
		// Output has to start first; stdin is unavailable until then.
		let mut output = vec![0u8; 1];
		output_read.read_exact(&mut output).await.unwrap();
		// Keep stdout blocked until a large stdin has been consumed.
		input_write.write_all(&vec![b'i'; 8192]).await.unwrap();
		input_write.shutdown().await.unwrap();
		output_read.read_to_end(&mut output).await.unwrap();
		output
	});
	let result = tokio::time::timeout(
		Duration::from_secs(2),
		pump_piped(hijacked.stream, input_read, output_write, tokio::io::sink()),
	)
	.await;
	if result.is_err() {
		peer.abort();
	}
	result
		.expect("stdin must advance while stdout is blocked")
		.unwrap();
	assert_eq!(peer.await.unwrap(), payload);
	assert_eq!(*received.lock().unwrap(), vec![b'i'; 8192]);
}

/// Model an async writer which schedules a write but only reports its
/// completion (or deferred failure) when flushed.
#[derive(Default)]
struct DeferredWriter {
	pending: Vec<u8>,
	delivered: Vec<u8>,
	flush_polls: usize,
	fail: bool,
}

impl tokio::io::AsyncWrite for DeferredWriter {
	fn poll_write(
		mut self: std::pin::Pin<&mut Self>,
		_: &mut std::task::Context<'_>,
		bytes: &[u8],
	) -> std::task::Poll<std::io::Result<usize>> {
		self.pending.extend_from_slice(bytes);
		std::task::Poll::Ready(Ok(bytes.len()))
	}

	fn poll_flush(
		mut self: std::pin::Pin<&mut Self>,
		cx: &mut std::task::Context<'_>,
	) -> std::task::Poll<std::io::Result<()>> {
		self.flush_polls += 1;
		if self.flush_polls == 1 {
			cx.waker().wake_by_ref();
			return std::task::Poll::Pending;
		}
		if self.fail {
			return std::task::Poll::Ready(Err(std::io::Error::other("deferred write failed")));
		}
		let pending = std::mem::take(&mut self.pending);
		self.delivered.extend_from_slice(&pending);
		std::task::Poll::Ready(Ok(()))
	}

	fn poll_shutdown(
		self: std::pin::Pin<&mut Self>,
		cx: &mut std::task::Context<'_>,
	) -> std::task::Poll<std::io::Result<()>> {
		tokio::io::AsyncWrite::poll_flush(self, cx)
	}
}

#[tokio::test]
async fn the_last_stdout_and_stderr_writes_are_flushed_before_success() {
	let (fake, _) = fake_session(vec![frame(1, b"last stdout"), frame(2, b"last stderr")]);
	let mut stdout = DeferredWriter::default();
	let mut stderr = DeferredWriter::default();
	pump_piped(
		hijack(&fake).await.stream,
		tokio::io::empty(),
		&mut stdout,
		&mut stderr,
	)
	.await
	.unwrap();
	assert_eq!(stdout.delivered, b"last stdout");
	assert_eq!(stderr.delivered, b"last stderr");
}

#[tokio::test]
async fn a_deferred_failure_of_either_output_stream_is_an_error() {
	for fail_stdout in [true, false] {
		let (fake, _) = fake_session(vec![frame(1, b"last stdout"), frame(2, b"last stderr")]);
		let mut stdout = DeferredWriter {
			fail: fail_stdout,
			..Default::default()
		};
		let mut stderr = DeferredWriter {
			fail: !fail_stdout,
			..Default::default()
		};
		let err = pump_piped(
			hijack(&fake).await.stream,
			tokio::io::empty(),
			&mut stdout,
			&mut stderr,
		)
		.await
		.expect_err("a deferred output failure must surface");
		assert!(
			matches!(err, crate::error::ComposeError::Io(ref e) if e.kind() == std::io::ErrorKind::Other)
		);
	}
}

#[tokio::test]
async fn eof_inside_any_header_or_payload_is_an_early_stream_end() {
	let complete = frame(1, b"complete");
	let incomplete = frame(2, b"unfinished");
	for cut in 1..incomplete.len() {
		let (fake, _) = fake_session(vec![complete.clone(), incomplete[..cut].to_vec()]);
		let mut stdout = Vec::new();
		let mut stderr = Vec::new();
		let err = pump_piped(
			hijack(&fake).await.stream,
			tokio::io::empty(),
			&mut stdout,
			&mut stderr,
		)
		.await
		.expect_err("EOF inside a frame must not discard buffered bytes");
		assert!(matches!(
			err,
			crate::error::ComposeError::Podman(crate::libpod::PodmanError::StreamEndedEarly)
		));
		assert_eq!(stdout, b"complete");
		assert!(stderr.is_empty());
	}
}

#[tokio::test]
async fn eof_between_complete_frames_including_empty_frames_is_success() {
	let (fake, _) = fake_session(vec![frame(1, b"complete"), frame(2, b"")]);
	let mut stdout = Vec::new();
	let mut stderr = Vec::new();
	pump_piped(
		hijack(&fake).await.stream,
		tokio::io::empty(),
		&mut stdout,
		&mut stderr,
	)
	.await
	.unwrap();
	assert_eq!(stdout, b"complete");
	assert!(stderr.is_empty());
}
