//! Unit tests for [`DrivenBody`]: every frame the inner body holds reaches
//! the caller, in order, whatever state the connection future is in (#1900).

use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::{BodyExt, Empty, StreamBody};
use hyper::body::Frame;
use hyper_util::rt::TokioIo;

use super::*;
use crate::libpod::parse_multiplexed;
use crate::libpod::types::stream::LogOutput;

type TestBody =
	StreamBody<futures_util::stream::Iter<std::vec::IntoIter<Result<Frame<Bytes>, hyper::Error>>>>;

/// A body that yields `chunks` as data frames, then ends.
fn body_from(chunks: Vec<Vec<u8>>) -> TestBody {
	let frames: Vec<Result<Frame<Bytes>, hyper::Error>> = chunks
		.into_iter()
		.map(|c| Ok(Frame::data(Bytes::from(c))))
		.collect();
	StreamBody::new(futures_util::stream::iter(frames))
}

fn noop_cx() -> Context<'static> {
	Context::from_waker(futures_util::task::noop_waker_ref())
}

/// A real `hyper::Error` (it has no public constructor): a request sent over
/// a socket whose peer is already gone.
async fn hyper_error() -> hyper::Error {
	let (client, server) = tokio::io::duplex(64);
	drop(server);
	let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(client))
		.await
		.unwrap();
	tokio::spawn(conn);
	sender
		.send_request(hyper::Request::new(Empty::<Bytes>::new()))
		.await
		.unwrap_err()
}

#[tokio::test]
async fn driven_body_drains_frames_after_the_connection_finished() {
	let conn: ConnectionFuture = Box::pin(async { Ok(()) });
	let driven = DrivenBody::new(
		body_from(vec![b"alpha".to_vec(), b"beta".to_vec()]),
		Some(conn),
	);
	let collected = driven.collect().await.unwrap().to_bytes();
	assert_eq!(collected.as_ref(), b"alphabeta");
}

#[test]
fn driven_body_yields_frames_while_the_connection_is_pending() {
	let conn: ConnectionFuture = Box::pin(std::future::pending());
	let mut driven = DrivenBody::new(
		body_from(vec![b"first".to_vec(), b"second".to_vec()]),
		Some(conn),
	);
	let mut cx = noop_cx();
	let mut out = Vec::new();
	for _ in 0..2 {
		match Pin::new(&mut driven).poll_frame(&mut cx) {
			Poll::Ready(Some(Ok(frame))) => out.extend_from_slice(&frame.into_data().unwrap()),
			other => panic!("a pending connection must not hold back a ready frame, got {other:?}"),
		}
	}
	assert_eq!(out, b"firstsecond");
}

/// hyper puts its own error into the body channel behind the frames it
/// already decoded. If `DrivenBody` returned the connection's error first,
/// those frames (the last lines before a broken stream) would be lost.
#[tokio::test]
async fn driven_body_keeps_buffered_frames_when_the_connection_fails() {
	let err = hyper_error().await;
	let conn: ConnectionFuture = Box::pin(async move { Err(err) });
	let driven = DrivenBody::new(
		body_from(vec![b"last".to_vec(), b"lines".to_vec()]),
		Some(conn),
	);
	let collected = driven
		.collect()
		.await
		.expect("the connection's error must not replace frames the body still holds")
		.to_bytes();
	assert_eq!(collected.as_ref(), b"lastlines");
}

/// Stdout and stderr frames, one line split across two frames, and an
/// unterminated tail, as `parse_multiplexed` must return them.
const EXPECTED: [(u8, &[u8]); 7] = [
	(1, b"hel"),
	(1, b"lo\n"),
	(1, b"after\n"),
	(2, b"err\n"),
	(1, b"world-"),
	(1, b"!\n"),
	(2, b"tail-no-newline"),
];

fn multiplexed_wire() -> Vec<u8> {
	let mut wire = Vec::new();
	for (stream, payload) in EXPECTED {
		wire.extend_from_slice(&[stream, 0, 0, 0]);
		wire.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
		wire.extend_from_slice(payload);
	}
	wire
}

/// The parser reassembles frames across every body chunking when the body
/// is a `DrivenBody`, so a frame boundary cannot hide behind one split.
#[tokio::test]
async fn parse_multiplexed_over_driven_body_is_split_independent() {
	let wire = multiplexed_wire();
	for chunk_size in [wire.len(), 16, 7, 3, 1] {
		let chunks = wire.chunks(chunk_size).map(<[u8]>::to_vec).collect();
		let conn: ConnectionFuture = Box::pin(std::future::pending());
		let mut stream = parse_multiplexed(DrivenBody::new(body_from(chunks), Some(conn)));
		let mut got = Vec::new();
		// Bounded so a body that stalls fails the test instead of hanging it.
		let drained = tokio::time::timeout(std::time::Duration::from_secs(5), async {
			while let Some(item) = stream.next().await {
				got.push(match item.unwrap() {
					LogOutput::StdOut { message } => (1, message.to_vec()),
					LogOutput::StdErr { message } => (2, message.to_vec()),
				});
			}
		})
		.await;
		assert!(
			drained.is_ok(),
			"chunk size {chunk_size}: the parsed stream stalled"
		);
		let want: Vec<(u8, Vec<u8>)> = EXPECTED.iter().map(|(s, p)| (*s, p.to_vec())).collect();
		assert_eq!(got, want, "chunk size {chunk_size}");
	}
}

#[test]
fn driven_body_meets_the_parser_bounds() {
	fn assert_bounds<T: Send + Unpin + 'static>() {}
	assert_bounds::<DrivenBody>();
}
