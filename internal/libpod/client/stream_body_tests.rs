use super::*;
use bytes::Bytes;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Frame;
use std::pin::Pin;
use std::task::{Context, Poll};

/// A connection future that finishes `Ok(())` immediately. Stands in for a
/// hyper driver that has already seen the response terminator and exited
/// cleanly. The body channel behind it still has frames, and the body
/// poll must surface them anyway.
fn ready_conn() -> ConnectionFuture {
	Box::pin(async { Ok(()) })
}

/// A connection future that never resolves. The body poll must still
/// surface any frames the body already has, in the order the body yields
/// them; the driver future being stuck must not be allowed to mask body
/// data.
fn pending_conn() -> ConnectionFuture {
	Box::pin(std::future::pending())
}

fn dummy_data(b: &[u8]) -> Frame<Bytes> {
	Frame::data(Bytes::copy_from_slice(b))
}

/// A body that emits a fixed list of frames, then ends. Synthesises what a
/// hyper Incoming body would yield on a closed connection.
fn body_from(
	chunks: Vec<Vec<u8>>,
) -> impl Body<Data = Bytes, Error = hyper::Error> + Send + Unpin + 'static {
	let stream = futures_util::stream::iter(
		chunks
			.into_iter()
			.map(|c| Ok::<Frame<Bytes>, hyper::Error>(dummy_data(&c))),
	);
	StreamBody::new(stream)
}

#[tokio::test]
async fn driven_body_surfaces_frames_when_connection_is_already_done() {
	// Connection returned Ready(Ok) before the body was polled: the body
	// channel may still have buffered frames, and the body poll must
	// deliver them anyway. Without this, a connection that finishes mid-
	// stream would silently swallow the tail of the response.
	let body = body_from(vec![b"alpha".to_vec(), b"beta".to_vec()]);
	let driven = DrivenBody::new(body, ready_conn());
	let collected = driven.collect().await.unwrap().to_bytes();
	assert_eq!(collected.as_ref(), b"alphabeta");
}

#[tokio::test]
async fn driven_body_surfaces_frames_when_connection_never_resolves() {
	// Connection stays pending forever: the body poll must not depend on
	// the connection making progress to deliver frames the body has
	// already pushed. This is the regression shape the inline driver
	// change must guard against.
	let body = body_from(vec![b"first".to_vec(), b"second".to_vec()]);
	let mut driven = DrivenBody::new(body, pending_conn());
	let mut cx = noop_cx();
	let mut out = Vec::new();
	for _ in 0..2 {
		match Pin::new(&mut driven).poll_frame(&mut cx) {
			Poll::Ready(Some(Ok(frame))) => {
				if let Ok(d) = frame.into_data() {
					out.extend_from_slice(&d);
				}
			}
			other => panic!("expected data frame, got {other:?}"),
		}
	}
	assert_eq!(out, b"firstsecond");
}

#[tokio::test]
async fn driven_body_is_unpin_and_send() {
	// DrivenBody is held inside `Response<DrivenBody>` across an
	// `.await`, so it must satisfy the bounds `parse_multiplexed` and
	// every other caller imposes on a streaming body. Send is required
	// because the parser is `Box<dyn Stream + Send>`.
	fn assert_unpin_send<T: Unpin + Send>() {}
	assert_unpin_send::<DrivenBody>();
	let body = body_from(Vec::new());
	let _driven = DrivenBody::new(body, pending_conn());
}

#[tokio::test]
async fn driven_body_collect_handles_split_frames() {
	// A body that arrives in two halves of the same logical chunk must
	// be reassembled into a single contiguous payload by `collect`. This
	// is the wire shape `into_body().collect()` callers rely on for
	// archives and JSON line streams.
	let body = body_from(vec![b"hel".to_vec(), b"lo".to_vec()]);
	let driven = DrivenBody::new(body, pending_conn());
	let collected = driven.collect().await.unwrap().to_bytes();
	assert_eq!(collected.as_ref(), b"hello");
}

#[tokio::test]
async fn driven_body_collect_after_dropped_connection_keeps_body() {
	// Drop the connection future out of the body and verify the body
	// still drains. The `Drop` for `DrivenBody` clears `conn`, so a
	// caller that drops a body (or its conn field) before collecting
	// must not leave the channel stuck. This pins the contract for the
	// `logs | head -1` early-close path: dropping the reader must drop
	// the connection without losing buffered frames the body has not
	// yielded yet.
	let body = body_from(vec![b"kept".to_vec()]);
	let mut driven = DrivenBody::new(body, pending_conn());
	driven.conn = None;
	let collected = driven.collect().await.unwrap().to_bytes();
	assert_eq!(collected.as_ref(), b"kept");
}

/// A no-op waker context for synchronous polling in tests.
fn noop_cx() -> Context<'static> {
	Context::from_waker(futures_util::task::noop_waker_ref())
}
