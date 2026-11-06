//! The cap on an archive that `cp` buffers before extracting it.

use bytes::Bytes;
use http_body_util::{Full, StreamBody};

use super::{buffer_archive, MAX_CP_ARCHIVE_BYTES};

fn body(len: usize) -> Full<Bytes> {
	Full::new(Bytes::from(vec![7u8; len]))
}

#[tokio::test]
async fn an_archive_of_exactly_the_cap_is_buffered() {
	let bytes = buffer_archive(body(4096), 4096).await.expect("at the cap");
	assert_eq!(bytes.len(), 4096);
}

#[tokio::test]
async fn an_archive_one_byte_over_the_cap_is_refused() {
	let error = buffer_archive(body(4097), 4096).await.expect_err("over");
	let text = error.to_string();
	assert!(text.contains("exceeds 4096 bytes"), "{text}");
}

/// A body that arrives in pieces and never says how long it is: the cap must
/// stop it at the piece that crosses the line, not after reading it all.
#[tokio::test]
async fn a_chunked_archive_is_cut_off_at_the_cap() {
	let piece = Bytes::from(vec![1u8; 1024]);
	let chunks = (0..1_000_000)
		.map(move |_| Ok::<_, std::convert::Infallible>(hyper::body::Frame::data(piece.clone())));
	let stream = StreamBody::new(futures_util::stream::iter(chunks));
	assert!(buffer_archive(stream, 8 * 1024).await.is_err());
}

#[test]
fn the_default_cap_stays_one_gibibyte() {
	assert_eq!(MAX_CP_ARCHIVE_BYTES, 1024 * 1024 * 1024);
}
