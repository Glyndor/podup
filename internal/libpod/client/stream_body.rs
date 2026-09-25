//! A response body whose underlying HTTP/1 connection is polled in the same
//! task as the body poll, so each new frame arrives without a cross-task
//! wake-up.
//!
//! The libpod client opens a connection through [`ConnPool::open_streaming`]
//! and normally hands the hyper driver to a `tokio::spawn`'d task. The driver
//! reads the socket, decodes HTTP frames, and pushes each one into an
//! internal mpsc channel that [`hyper::body::Incoming`] consumes. Every push
//! wakes the reading task across a `futex`. Measured on the flood-flood-1
//! fixture (2 097 152 empty lines plus `seq 1 20000`, 2 117 152 multiplexed
//! frames, 23 377 566 bytes): that wake-up cost about 6 M futex calls and
//! 18 s of sys time on top of the raw socket read, taking `podup logs`
//! from the curl time of ~10 s to ~20 s.
//!
//! Holding the connection future in the same task as the body poll removes
//! that wake-up: the connection poll and the body poll land in the same
//! task, frames the connection just pushed are visible to the body poll
//! without a context switch, and the parser drains them in one trip.
//!
//! The body is `Send + Unpin + 'static` (the inner `Incoming` is wrapped in
//! `Pin<Box<_>>` so the wrapper stays `Unpin` and can be polled without
//! `pin-project`), implements `Body<Data = Bytes, Error = hyper::Error>` so
//! every caller of the streaming methods keeps working unchanged
//! (`into_body()` + `frame().await`, `body.collect()`, the multiplexed
//! parser), and drops the connection on its own drop so a caller that
//! abandons the stream still closes the underlying socket.

use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use hyper::body::{Body, Frame, Incoming, SizeHint};

/// HTTP/1 connection future whose driver task this body replaces. Same type
/// the streaming pool open path used to hand to `tokio::spawn`, kept by value
/// here so its drop closes the IO.
pub type ConnectionFuture =
	Pin<Box<dyn std::future::Future<Output = Result<(), hyper::Error>> + Send>>;

/// A body that polls the connection in line with `Incoming`. Drop closes
/// the connection.
pub struct DrivenBody {
	inner: Pin<Box<Incoming>>,
	conn: Option<ConnectionFuture>,
}

impl DrivenBody {
	/// Build a body that drives `conn` in the same task as each
	/// [`Body::poll_frame`] call.
	pub fn new(body: Incoming, conn: ConnectionFuture) -> Self {
		Self {
			inner: Box::pin(body),
			conn: Some(conn),
		}
	}
}

impl Body for DrivenBody {
	type Data = Bytes;
	type Error = hyper::Error;

	fn poll_frame(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		// Drive the connection first, in the same task, so a frame the
		// connection has just decoded into the body channel is visible to the
		// body poll without a wake-up.
		if let Some(conn) = self.conn.as_mut() {
			match conn.as_mut().poll(cx) {
				Poll::Ready(Ok(())) => {
					// Connection reported a clean close. The body channel may
					// still have buffered frames; keep polling `inner` until it
					// returns `None`.
					self.conn = None;
				}
				Poll::Ready(Err(e)) => {
					self.conn = None;
					return Poll::Ready(Some(Err(e)));
				}
				Poll::Pending => {}
			}
		}
		self.inner.as_mut().poll_frame(cx)
	}

	fn is_end_stream(&self) -> bool {
		self.inner.is_end_stream()
	}

	fn size_hint(&self) -> SizeHint {
		self.inner.size_hint()
	}
}

impl Drop for DrivenBody {
	fn drop(&mut self) {
		// Dropping the connection future drops the underlying IO, which
		// hyper reports to Podman as a socket close. A caller that bails out
		// mid-stream therefore still frees the daemon-side resource; the
		// previous design relied on `StreamingConn::drop` aborting the
		// driver task to achieve the same.
		self.conn = None;
	}
}
