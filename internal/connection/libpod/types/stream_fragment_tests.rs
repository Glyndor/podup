//! A record that reaches the parser in many small pieces is searched for its
//! newline once, not once per piece.

use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::StreamBody;
use hyper::body::Frame;

use super::{parse_json_lines, EXAMINED};

type Chunks = Vec<Result<Frame<Bytes>, hyper::Error>>;

fn one_byte_frames(data: &[u8]) -> Chunks {
	data.iter()
		.map(|b| Ok(Frame::data(Bytes::copy_from_slice(&[*b]))))
		.collect()
}

fn examined() -> u64 {
	EXAMINED.with(std::cell::Cell::get)
}

/// A valid record of about 16 KiB delivered a byte at a time comes out as the
/// same value as when it is delivered whole, and the newline search looks at
/// a number of bytes proportional to its size. Counting bytes keeps the check
/// independent of how fast the machine is.
#[tokio::test]
async fn a_long_record_in_one_byte_frames_is_searched_in_linear_time() {
	const LEN: usize = 16 * 1024;
	let mut record = br#"{"stream":""#.to_vec();
	record.resize(record.len() + LEN, b'x');
	record.extend_from_slice(b"\"}\n");

	let before = examined();
	let body = StreamBody::new(futures_util::stream::iter(one_byte_frames(&record)));
	let mut stream = parse_json_lines::<serde_json::Value, _>(body);
	let item = stream.next().await.expect("one record").expect("parses");
	assert!(stream.next().await.is_none());
	let looked_at = examined() - before;

	assert_eq!(item["stream"].as_str().map(str::len), Some(LEN));
	assert!(
		looked_at <= 2 * record.len() as u64,
		"{looked_at} bytes searched for a {} byte record",
		record.len()
	);
}

/// A line that never gets a newline is searched once too, and the stream that
/// ends on it reports the early end, not a parse error.
#[tokio::test]
async fn a_line_without_a_newline_is_searched_in_linear_time() {
	let junk = vec![b'x'; 16 * 1024];
	let before = examined();
	let body = StreamBody::new(futures_util::stream::iter(one_byte_frames(&junk)));
	let mut stream = parse_json_lines::<serde_json::Value, _>(body);
	let err = stream.next().await.expect("an item").expect_err("not JSON");
	assert!(
		matches!(err, crate::libpod::error::PodmanError::StreamEndedEarly),
		"{err}"
	);
	assert!(examined() - before <= 2 * junk.len() as u64);
}
