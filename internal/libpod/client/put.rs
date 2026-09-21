use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Frame;
use hyper::Method;

use super::{full, Client, Result, READ_TIMEOUT};

impl Client {
	/// `PUT` with raw bytes body → expect 2xx.
	pub async fn put_bytes_ok(&self, path: &str, bytes: Bytes, content_type: &str) -> Result<()> {
		let len = bytes.len();
		let req = Self::build_request(Method::PUT, path, full(bytes), Some(content_type))?;
		let resp = match self.send(req, Some(READ_TIMEOUT)).await {
			Ok(r) => r,
			Err(e) => {
				tracing::debug!(
					"PUT {path} ({content_type}, {len} bytes) ended [{}]: {e}",
					e.stream_end_kind()
				);
				return Err(e);
			}
		};
		let (status, body) = resp.read_body(Some(READ_TIMEOUT)).await?;
		Self::check_status(status, &body)
	}

	/// `PUT` with raw bytes body, advancing `counter` as bytes flow →
	/// expect 2xx. The body is emitted in `PUT_CHUNK_SIZE`-byte frames
	/// so the counter advances across the PUT rather than jumping by
	/// the full size the moment hyper polls `Full<Bytes>`. This is the
	/// `cp` upload path; every other caller goes through `put_bytes_ok`.
	pub async fn put_bytes_ok_counting(
		&self,
		path: &str,
		bytes: Bytes,
		content_type: &str,
		counter: Arc<AtomicU64>,
	) -> Result<()> {
		let stream = futures_util::stream::unfold((0usize, bytes), move |(offset, bytes)| {
			let counter = counter.clone();
			async move {
				if offset >= bytes.len() {
					return None;
				}
				let end = (offset + PUT_CHUNK_SIZE).min(bytes.len());
				let chunk = bytes.slice(offset..end);
				counter.fetch_add(chunk.len() as u64, Ordering::Relaxed);
				Some((Ok::<_, std::io::Error>(Frame::data(chunk)), (end, bytes)))
			}
		});
		let body = http_body_util::StreamBody::new(stream).boxed_unsync();
		let req = Self::build_request(Method::PUT, path, body, Some(content_type))?;
		let resp = match self.send(req, Some(READ_TIMEOUT)).await {
			Ok(r) => r,
			Err(e) => {
				tracing::debug!(
					"PUT {path} ({content_type}, counting) ended [{}]: {e}",
					e.stream_end_kind()
				);
				return Err(e);
			}
		};
		let (status, body) = resp.read_body(Some(READ_TIMEOUT)).await?;
		Self::check_status(status, &body)
	}
}

/// Size of each frame the `put_bytes_ok_counting` body emits. Large
/// enough that the per-frame overhead (one, two hyper polls per frame)
/// is in the noise, small enough that a 220 MB PUT updates the byte
/// counter hundreds of times before it lands.
const PUT_CHUNK_SIZE: usize = 64 * 1024;
