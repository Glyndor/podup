//! Transport-level helpers the client uses on every request: the boxed body
//! type, the buffered-body constructor, the timeouts that bound waiting on the
//! Podman socket, the close-detection helper, and the generic
//! [`apply_timeout`](super::Client::apply_timeout) wrapper.
//!
//! Split from `mod.rs` so the per-file code-line cap is respected while the
//! items this file owns (shared by every request path) stay in one place.

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::Response;

use super::{Client, PodmanError, Result};

/// The request body every call shares. A boxed body so a fully-buffered
/// `Full<Bytes>` (almost every call) and a lazily-streamed build-context body
/// (the `build` endpoint) travel the same client path. `Unsync` because hyper's
/// `send_request` only requires the body to be `Send`, and the streamed body is
/// not `Sync`.
pub(crate) type BoxBody = http_body_util::combinators::UnsyncBoxBody<Bytes, std::io::Error>;

/// Box a fully-buffered byte payload into [`BoxBody`](super::BoxBody). `Full`'s
/// error is `Infallible`, mapped to the unified `io::Error` (which it never
/// produces).
pub(crate) fn full(bytes: Bytes) -> BoxBody {
	http_body_util::Full::new(bytes)
		.map_err(|never| match never {})
		.boxed_unsync()
}

/// Upper bound on a buffered (non-streaming) response body. Caps memory use
/// when the daemon returns an oversized or runaway response.
pub(crate) const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

/// Ceiling on establishing the socket connection and HTTP handshake. Bounds the
/// wait when the Podman socket is absent, busy, or unresponsive. This times the
/// connect only; it does not limit the duration of a streaming response body.
pub(crate) const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Ceiling on reading a *buffered* (non-streaming) response body. Without it a
/// daemon that accepts the request, sends headers, then stalls would hang the
/// CLI forever. Streaming helpers (logs, attach, archive) are deliberately not
/// bounded by this; they are long-lived by design.
pub(crate) const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Whether a response carried `Connection: close`. When set, the socket is
/// unusable for any further request and the pool must discard it instead of
/// handing it back to the next acquirer. HTTP/1.1 keep-alive is the default
/// in podup's real wire path; a `close` value is the server telling us this
/// socket is single-use.
pub(crate) fn has_connection_close(resp: &Response<Incoming>) -> bool {
	resp.headers()
		.get(hyper::header::CONNECTION)
		.and_then(|v| v.to_str().ok())
		.map(|v| v.split(',').any(|t| t.trim().eq_ignore_ascii_case("close")))
		.unwrap_or(false)
}

impl Client {
	/// Await `fut`, optionally bounded by `timeout`.
	pub(super) async fn apply_timeout<F, T>(
		timeout: Option<std::time::Duration>,
		phase: &str,
		fut: F,
	) -> Result<T>
	where
		F: std::future::Future<Output = T>,
	{
		match timeout {
			Some(limit) => tokio::time::timeout(limit, fut)
				.await
				.map_err(|_| PodmanError::Api {
					status: 0,
					message: format!("timed out after {}s {phase}", limit.as_secs()),
				}),
			None => Ok(fut.await),
		}
	}
}
