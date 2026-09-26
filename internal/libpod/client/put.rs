use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Frame;
use hyper::Method;

use super::{Client, Result, READ_TIMEOUT};
use futures_util::Stream;

impl Client {
	/// `PUT` with a streamed body → expect 2xx. The body is the caller's
	/// frame stream: each `Frame<Bytes>` is one chunk the libpod endpoint
	/// will read in turn. A mid-pack error surfaces as an `Err` item in the
	/// stream, which hyper reports as a body write failure; the caller is
	/// responsible for surfacing the original error from the producer side
	/// rather than relying on the transport to deliver it.
	///
	/// The `cp` upload path uses this so the archive is no longer buffered
	/// as one `Vec<u8>` before the PUT. Memory is bounded by the caller's
	/// stream's own bound, not the archive's size.
	pub async fn put_stream_ok<S>(&self, path: &str, chunks: S, content_type: &str) -> Result<()>
	where
		S: Stream<Item = std::result::Result<Frame<Bytes>, std::io::Error>> + Send + 'static,
	{
		let body = http_body_util::StreamBody::new(chunks).boxed_unsync();
		let req = Self::build_request(Method::PUT, path, body, Some(content_type))?;
		let resp = match self.send(req, Some(READ_TIMEOUT)).await {
			Ok(r) => r,
			Err(e) => {
				tracing::debug!(
					"PUT {path} ({content_type}, streamed) ended [{}]: {e}",
					e.stream_end_kind()
				);
				return Err(e);
			}
		};
		let (status, body) = resp.read_body(Some(READ_TIMEOUT)).await?;
		Self::check_status(status, &body)
	}
}
