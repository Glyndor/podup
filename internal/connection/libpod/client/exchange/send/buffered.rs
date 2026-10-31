//! The buffered request path: acquire a pooled connection, send one request,
//! and pair the response with the pool guard that has to stay alive until the
//! body is drained.
//!
//! Split from `mod.rs` so [`Client::send`] and [`BufferedResponse`] sit next to
//! each other, with the close-detection and timeout wiring they share.

use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};

use super::{has_connection_close, BoxBody, PoolGuard, CONNECT_TIMEOUT};
use super::{Client, Result};

/// A buffered HTTP response paired with the pool guard that keeps the
/// underlying HTTP/1.1 connection checked out until the body is fully
/// drained. The guard is private, so the only way to release it is to call
/// [`Client::read_body`], which consumes the response and the guard
/// together. Returning the response bare (without the guard) was the
/// original bug: between `send` and `read_body` the guard dropped, the
/// connection landed back in the idle queue, and the next acquirer could
/// write a new request to the same socket while the previous body was
/// still arriving. With `Transfer-Encoding: chunked` (or any framing where
/// the body length is not declared up front in the headers) that
/// interleaves the two requests on the wire and the parser sees garbage
/// (#1740).
pub(crate) struct BufferedResponse {
	// Held by name only; never read. Its drop runs at the end of
	// `read_body`'s scope, AFTER the body has been drained. Drop order in
	// Rust is reverse declaration order, so the body is dropped first and
	// the guard last.
	_guard: PoolGuard,
	resp: Response<Incoming>,
}

impl BufferedResponse {
	/// The response headers, borrowed without releasing the guard. Callers
	/// that branch on status or read a response-specific header must still
	/// call [`BufferedResponse::read_body`] on the returned value to release
	/// the connection cleanly.
	pub(in crate::connection::libpod::client) fn headers(&self) -> &hyper::HeaderMap {
		self.resp.headers()
	}

	/// Read the full response body off the buffered connection. Consumes
	/// `self` so the underlying pool guard (and thus the HTTP/1.1
	/// connection) is released only after the body is drained. A caller
	/// that drops the [`BufferedResponse`] without calling `read_body` is
	/// a bug: the body may still be arriving, the connection is not safe
	/// to reuse, and the next caller will see interleaved bytes (#1740).
	pub(in crate::connection::libpod::client) async fn read_body(
		self,
		read_timeout: Option<std::time::Duration>,
	) -> Result<(StatusCode, Vec<u8>)> {
		Client::read_response_body::<Incoming>(self.resp, read_timeout).await
	}
}

impl Client {
	/// Send a request and return the buffered response.
	///
	/// `response_timeout` bounds how long we wait for the server to return the
	/// response head. Pass `Some` (the default
	/// [`READ_TIMEOUT`](crate::libpod::client::request::transport::READ_TIMEOUT))
	/// for ordinary and streaming calls, where the head arrives promptly; this
	/// stops a socket that accepts the connection but never replies from hanging
	/// the CLI indefinitely.
	/// Pass `None` only for endpoints that legitimately block server-side before
	/// the head (e.g. `wait?condition=stopped`), whose callers impose an outer
	/// budget.
	///
	/// Returns a [`BufferedResponse`] rather than a bare `Response<Incoming>`
	/// so the pool guard that holds the HTTP/1.1 connection checked out is
	/// kept alive across the body read. Releasing the guard before the body
	/// is drained lets the next acquirer write a new request to the same
	/// socket while the previous body is still arriving; with chunked
	/// transfer encoding (or any framing where the body length is not known
	/// up front in the headers) that interleaves the two requests on the
	/// wire and the parser sees garbage (#1740). The guard is private to
	/// [`BufferedResponse`], so callers cannot release it without first
	/// reading the body.
	pub(in crate::connection::libpod::client) async fn send(
		&self,
		req: Request<BoxBody>,
		response_timeout: Option<std::time::Duration>,
	) -> Result<BufferedResponse> {
		tracing::debug!("libpod {} {}", req.method(), req.uri().path());
		let mut guard = tokio::time::timeout(CONNECT_TIMEOUT, self.pool.acquire())
			.await
			.map_err(|_| super::PodmanError::Api {
				status: 0,
				message: format!(
					"timed out after {}s connecting to the Podman socket",
					CONNECT_TIMEOUT.as_secs()
				),
			})??;
		let request = guard.sender_mut().send_request(req);
		let send_result = Self::apply_timeout(
			response_timeout,
			"waiting for the Podman socket to respond",
			request,
		)
		.await;
		match send_result {
			Ok(Ok(resp)) => {
				if has_connection_close(&resp) {
					guard.poison();
				}
				Ok(BufferedResponse {
					_guard: guard,
					resp,
				})
			}
			Ok(Err(e)) => {
				guard.poison();
				Err(super::PodmanError::Hyper(e))
			}
			Err(e) => {
				guard.poison();
				Err(e)
			}
		}
	}
}
