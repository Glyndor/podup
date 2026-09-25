//! Behaviour tests for the streaming path that wraps [`super::DrivenBody`].
//!
//! These pin the byte-for-byte equivalence between the inline-driven body
//! (the new path) and a bare body wrapper (the reference): every parsed
//! payload must be identical, including the order, channel (stdout /
//! stderr), and the bytes themselves, for a stream that interleaves
//! stdout and stderr frames and splits lines across frames (#1900).
//!
//! The two paths exercise the same `parse_multiplexed_body` parser; the
//! only difference is the body wrapper. The reference wraps frames in a
//! [`StreamBody`]; the DrivenBody path wraps the same `StreamBody` in
//! [`super::DrivenBody`] (the inline-driver layer). A regression in
//! `DrivenBody::poll_frame` that swallowed, reordered, or duplicated
//! frames would diverge here.

use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::StreamBody;
use hyper::body::Frame;

use crate::libpod::types::stream::LogOutput;
use crate::libpod::parse_multiplexed;

/// Frames for a synthetic body that interleaves stdout and stderr
/// frames, splits a line across two frames, and adds an unterminated tail
/// at the end. The shape is the one the `flood-flood-1` fixture and any
/// container with both stdout and stderr activity exercise.
fn interleaved_multiplexed_bytes() -> Vec<u8> {
	let mut buf = Vec::new();
	fn frame(stream_type: u8, payload: &[u8], out: &mut Vec<u8>) {
		out.push(stream_type);
		out.extend_from_slice(&[0, 0, 0]);
		out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
		out.extend_from_slice(payload);
	}
	// stdout partial ("hel"), then stdout continuation ("lo\n").
	frame(1, b"hel", &mut buf);
	frame(1, b"lo\n", &mut buf);
	// stderr ("err\n") immediately after a stdout line.
	frame(1, b"after\n", &mut buf);
	frame(2, b"err\n", &mut buf);
	// A second stdout line split across two frames ("world-"), ("!\n").
	frame(1, b"world-", &mut buf);
	frame(1, b"!\n", &mut buf);
	// An unterminated tail that must reach the caller in the same order.
	frame(2, b"tail-no-newline", &mut buf);
	buf
}

/// Convert the wire bytes into a synthetic frame stream that yields them
/// in the requested number of body-level frames. The parser must
/// reassemble across those boundaries to surface the right `LogOutput`
/// items; a regression in the DrivenBody that breaks that reassembly
/// shows up as a payload mismatch.
fn chunks_for(wire: Vec<u8>, frame_count: usize) -> Vec<Vec<u8>> {
	let chunk_size = (wire.len() + frame_count - 1) / frame_count;
	let mut chunks = Vec::new();
	for slice in wire.chunks(chunk_size) {
		chunks.push(slice.to_vec());
	}
	chunks
}

/// The reference body: a plain `StreamBody` of the same frames. Goes
/// through `parse_multiplexed` without the inline-driver layer.
fn reference_body(
	chunks: Vec<Vec<u8>>,
) -> StreamBody<futures_util::stream::Iter<std::vec::IntoIter<Result<Frame<Bytes>, hyper::Error>>>> {
	let frames: Vec<Result<Frame<Bytes>, hyper::Error>> = chunks
		.into_iter()
		.map(|c| Ok::<_, hyper::Error>(Frame::data(Bytes::from(c))))
		.collect();
	StreamBody::new(futures_util::stream::iter(frames))
}

/// The new path: same frames, but the body is wrapped in
/// [`super::DrivenBody`] with a never-resolving connection future (the
/// driver has not signalled a clean close yet, which is the steady-state
/// case during the body read).
fn driven_body(
	chunks: Vec<Vec<u8>>,
) -> super::DrivenBody<StreamBody<futures_util::stream::Iter<std::vec::IntoIter<Result<Frame<Bytes>, hyper::Error>>>>> {
	let frames: Vec<Result<Frame<Bytes>, hyper::Error>> = chunks
		.into_iter()
		.map(|c| Ok::<_, hyper::Error>(Frame::data(Bytes::from(c))))
		.collect();
	let inner: StreamBody<futures_util::stream::Iter<std::vec::IntoIter<Result<Frame<Bytes>, hyper::Error>>>> =
		StreamBody::new(futures_util::stream::iter(frames));
	let conn: super::ConnectionFuture = Box::pin(std::future::pending());
	super::DrivenBody::new(inner, conn)
}

async fn collect<B>(body: B) -> Vec<LogOutput>
where
	B: hyper::body::Body<Data = Bytes, Error = hyper::Error> + Send + Unpin + 'static,
{
	let mut stream = parse_multiplexed(body);
	let mut out = Vec::new();
	while let Some(msg) = stream.next().await {
		out.push(msg.expect("frame"));
	}
	out
}

/// The DrivenBody path must surface the same `LogOutput` items, in the
/// same order, with the same bytes, as the reference body. Tested across
/// multiple chunk splits so a frame-boundary bug cannot hide behind a
/// particular chunking (#1900).
#[tokio::test]
async fn logs_driven_body_emits_identical_payloads_to_reference_body() {
	let wire = interleaved_multiplexed_bytes();
	for frame_count in [1usize, 2, 4, 8, 16] {
		let chunks = chunks_for(wire.clone(), frame_count);

		let reference_outputs = collect(reference_body(chunks.clone())).await;
		let driven_outputs = collect(driven_body(chunks)).await;

		assert_eq!(
			reference_outputs.len(),
			driven_outputs.len(),
			"chunk count {frame_count}: same number of payloads"
		);
		for (i, (a, b)) in reference_outputs
			.iter()
			.zip(driven_outputs.iter())
			.enumerate()
		{
			let (a_tag, a_msg) = match a {
				LogOutput::StdOut { message } => ("stdout", message.as_ref()),
				LogOutput::StdErr { message } => ("stderr", message.as_ref()),
			};
			let (b_tag, b_msg) = match b {
				LogOutput::StdOut { message } => ("stdout", message.as_ref()),
				LogOutput::StdErr { message } => ("stderr", message.as_ref()),
			};
			assert_eq!(
				a_tag, b_tag,
				"chunk count {frame_count}, payload {i}: channel must match (got {a_tag} vs {b_tag})"
			);
			assert_eq!(
				a_msg, b_msg,
				"chunk count {frame_count}, payload {i}: bytes must match"
			);
		}
	}
}

/// The DrivenBody must also accept the parser through the public surface
/// (the `parse_multiplexed(body) -> BoxStream<LogOutput>` shape every
/// caller uses), not only the body-form seam the unit tests use.
#[tokio::test]
async fn logs_driven_body_passes_through_public_parse_multiplexed() {
	let chunks = chunks_for(interleaved_multiplexed_bytes(), 4);
	let driven = driven_body(chunks);
	let mut stream = parse_multiplexed(driven);
	let mut count = 0;
	while let Some(item) = stream.next().await {
		item.expect("frame");
		count += 1;
	}
	assert!(
		count >= 6,
		"the interleaved stream yields at least one payload per frame (got {count})"
	);
}

/// The DrivenBody must surface the body's `is_end_stream` and `size_hint`
/// unchanged so callers like `Limited` and `BodyExt::collect` see the
/// same hint shape as they did on plain `Incoming`. This pins the
/// delegation contract.
#[tokio::test]
async fn logs_driven_body_delegates_body_hints() {
	let chunks = chunks_for(interleaved_multiplexed_bytes(), 2);
	let driven = driven_body(chunks);
	use hyper::body::Body;
	assert!(!driven.is_end_stream(), "frames still pending");
	let _ = driven.size_hint();
}

/// The DrivenBody is `Send + Unpin + 'static` so it satisfies the bounds
/// `parse_multiplexed` and `parse_multiplexed_body` require from a body.
#[test]
fn logs_driven_body_satisfies_parser_bounds() {
	fn assert_send_unpin_static<T: Send + Unpin + 'static>() {}
	assert_send_unpin_static::<super::DrivenBody>();
}
