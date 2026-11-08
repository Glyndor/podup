//! Test-only fake Podman socket.
//!
//! A minimal libpod HTTP responder bound to a Unix domain socket, so
//! lifecycle/scale/query tests can assert exit-code semantics against canned
//! API responses without a real Podman daemon. [`Client`] opens a fresh
//! connection per request (see `internal/connection/libpod/client/mod.rs`), so this only
//! ever needs to answer one HTTP/1.1 request per accepted connection, with no
//! keep-alive.
//!
//! It does frame a chunked body, for one reason: a stream that ends *badly* has
//! a wire shape, and podup's central open question about streaming (#1104) is
//! whether it can tell that shape from a stream that ended well. A real Podman
//! cannot be asked to break a stream on demand, and which shape it produces at a
//! CLEAN end turns out to differ by version, so the two cases are pinned here,
//! deterministically, with no daemon and no version in the picture.

#![cfg(unix)]

use std::sync::{Arc, Mutex};

use tokio::net::UnixListener;
use tokio::task::JoinHandle;

use crate::libpod::Client;

mod handlers;

/// What the fake writes back for one request.
pub(super) enum FakeReply {
	/// A `content-length`-framed body, closed cleanly. What every routing test
	/// that only cares about status codes wants.
	Body(u16, String),
	/// A 200 with a **chunked** body: each entry is written as one chunk, then
	/// the terminating zero-length chunk is sent. This is a stream that ends the
	/// way HTTP says it should.
	ChunkedEnd(Vec<String>),
	/// A 200 with a **chunked** body that stops between chunks: each entry is
	/// written as one complete chunk and then the connection is closed with NO
	/// terminating chunk. A stream that died where a chunk header should start.
	ChunkedTruncated(Vec<String>),
	/// A 200 with a **chunked** body cut in the middle of a chunk's payload: the
	/// header promises more bytes than are then written, and the connection
	/// closes. The other place a severed stream can land and, measured, hyper
	/// classifies the two differently, which is why both exist here.
	ChunkedCutMidPayload(String),
	/// The request is read and accepted, and then the connection closes with **no
	/// response at all**, not even a status line.
	///
	/// This is the shape `PodmanError::is_incomplete_message` names: hyper's
	/// `IncompleteMessage` is about the message *head*, so it is the one reply
	/// here that produces it, and the severed-body variants above do not.
	///
	/// It is what libpod does on Podman 6 to the container-archive PUT (#1097,
	/// applies the archive then hangs up) and to state-changing POSTs under
	/// concurrency (#1339). Both are handled by re-checking the observable out of
	/// band, and until this existed neither discriminator had a test that could
	/// reach it.
	ClosedWithoutResponse,
	/// The request is read and accepted, and the connection is **held open with
	/// no response at all**, for a bounded time.
	///
	/// This is the shape of a daemon that takes a request and never answers it:
	/// the client blocks on the read until its own deadline ends the wait.
	/// Nothing on the wire tells it apart from a daemon that is merely slow,
	/// which is exactly why the client needs a deadline rather than trusting the
	/// peer to answer.
	Silent,
	/// A status line and the given headers, with no body. What a `HEAD` gets.
	///
	/// The container-archive stat lives entirely in a response header
	/// (`X-Docker-Container-Path-Stat`), which `Body` has no way to send, so
	/// the re-verification that follows a dropped archive PUT could not be
	/// answered from here until this existed.
	Headers(u16, Vec<(&'static str, String)>),
	/// A 200 whose connection is hijacked: the head is written, then the
	/// frames, and the connection stays open so the client can keep sending
	/// on it. That is what an exec session with an attached stdin is: the
	/// client's bytes travel up the same connection its output comes down
	/// (measured against Podman 5.7.0, `POST /libpod/exec/{id}/start`).
	///
	/// The bytes the client sends after the head are recorded, so a test can
	/// assert that the caller's input really reached the session.
	Hijacked {
		/// The response head, written verbatim before the frames.
		head: String,
		/// The bytes written after the head, as the session's output.
		frames: Vec<Vec<u8>>,
		/// Where the bytes the client sends after the head are recorded.
		received: Arc<Mutex<Vec<u8>>>,
	},
}

/// A test's routing rule: `(method, target) -> reply`, where `target` is the
/// request path plus its raw query string (e.g.
/// `/v5.0.0/libpod/containers/proj-web-1/start`).
pub(super) type Responder = dyn Fn(&str, &str) -> FakeReply + Send + Sync;

/// A fake libpod socket driven by a routing closure; see [`Responder`].
pub(super) struct FakePodman {
	sock_path: std::path::PathBuf,
	/// Every request answered so far, as `"METHOD target"`, letting a test assert
	/// that a best-effort pass attempted every container even after one of them
	/// failed.
	pub(super) requests: Arc<Mutex<Vec<String>>>,
	/// The raw body bytes for every request answered so far, in the same order
	/// as [`requests`](Self::requests). Empty for requests with no body or that
	/// did not advertise a `Content-Length`. Tests that need to assert on the
	/// JSON payload of a `POST` (e.g. the `SpecGenerator` sent to
	/// `/containers/create`) read this.
	pub(super) bodies: Arc<Mutex<Vec<Vec<u8>>>>,
	/// The `Content-Type` header of every request answered so far, in the same
	/// order as [`requests`](Self::requests). Empty when the request carried
	/// no such header. The build request is the one that needs it: the
	/// endpoint reads a multipart body only when the header says so.
	pub(super) content_types: Arc<Mutex<Vec<String>>>,
	_dir: tempfile::TempDir,
	task: JoinHandle<()>,
}

impl FakePodman {
	/// A fresh [`Client`] pointed at this fake socket. [`Client`] is a thin,
	/// stateless per-request handle (see `internal/connection/libpod/client/mod.rs`), so a
	/// new one is created on every call rather than shared/cloned.
	pub(super) fn client(&self) -> Client {
		Client::new(self.sock_path.to_string_lossy().into_owned())
	}
}

impl Drop for FakePodman {
	fn drop(&mut self) {
		// Stop accepting new connections; in-flight ones simply finish or are
		// dropped along with the temp dir (the test is already done with them).
		self.task.abort();
	}
}

/// Start a fake Podman socket that answers every request via `respond`.
/// Connection-per-request, matching how [`Client`] talks to the real daemon.
pub(super) fn start<F>(respond: F) -> FakePodman
where
	F: Fn(&str, &str) -> (u16, String) + Send + Sync + 'static,
{
	start_replying(move |method, target| {
		let (status, body) = respond(method, target);
		FakeReply::Body(status, body)
	})
}

/// As [`start`], but the routing closure chooses the wire shape too, including
/// a chunked body that ends cleanly or one that is cut off mid-stream.
pub(super) fn start_replying<F>(respond: F) -> FakePodman
where
	F: Fn(&str, &str) -> FakeReply + Send + Sync + 'static,
{
	let dir = tempfile::tempdir().expect("create temp dir for fake podman socket");
	let sock_path = dir.path().join("podman.sock");
	let listener = UnixListener::bind(&sock_path).expect("bind fake podman socket");
	let respond: Arc<Responder> = Arc::new(respond);
	let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
	let bodies: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
	let content_types: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

	let task_requests = requests.clone();
	let task_bodies = bodies.clone();
	let task_content_types = content_types.clone();
	let task = tokio::spawn(async move {
		loop {
			let Ok((stream, _)) = listener.accept().await else {
				break;
			};
			let respond = respond.clone();
			let requests = task_requests.clone();
			let bodies = task_bodies.clone();
			let content_types = task_content_types.clone();
			tokio::spawn(async move {
				let _ = handlers::serve_one(
					stream,
					respond.as_ref(),
					&requests,
					&bodies,
					&content_types,
				)
				.await;
			});
		}
	});

	FakePodman {
		sock_path,
		requests,
		bodies,
		content_types,
		_dir: dir,
		task,
	}
}
