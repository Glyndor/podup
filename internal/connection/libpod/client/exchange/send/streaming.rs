//! The streaming request path: open a dedicated connection, send one request,
//! and hand the response back with a
//! [`DrivenBody`](crate::libpod::client::stream::stream_body::DrivenBody) that
//! drives the HTTP/1 connection inline with the body poll. Also holds the
//! shared body-read helper ([`Client::read_response_body`]) used by
//! [`BufferedResponse::read_body`](super::buffered::BufferedResponse::read_body)
//! and the streaming error path ([`Client::stream_or_err`]).
//!
//! Split from `mod.rs` so the long-lived connection shape and the
//! size-capped read helper sit together.

use bytes::Bytes;
use futures_util::Future;
use http_body_util::{BodyExt, Limited};
use hyper::{Request, Response, StatusCode};

use super::{BoxBody, ConnState, DrivenBody, CONNECT_TIMEOUT, MAX_RESPONSE_BYTES, READ_TIMEOUT};
use super::{Client, PodmanError, Result};

impl Client {
	/// Send a request whose response body is a long-lived stream and return
	/// the raw response. The connection is opened outside the buffered pool;
	/// the response body owns the HTTP/1 driver so the reading task can poll
	/// it in line with the body and skip the cross-task wake-up per frame
	/// (#1900).
	pub(in crate::connection::libpod::client) async fn send_streaming(
		&self,
		req: Request<BoxBody>,
		response_timeout: Option<std::time::Duration>,
	) -> Result<Response<DrivenBody>> {
		tracing::debug!("libpod {} {}", req.method(), req.uri().path());
		let conn = tokio::time::timeout(CONNECT_TIMEOUT, self.pool.open_streaming())
			.await
			.map_err(|_| PodmanError::Api {
				status: 0,
				message: format!(
					"timed out after {}s connecting to the Podman socket",
					CONNECT_TIMEOUT.as_secs()
				),
			})??;
		let (mut sender, driver) = conn.into_parts();
		// The sender future awaits a response on a oneshot the dispatcher
		// fills. Nobody else drives the dispatcher, so we have to poll it in
		// the same task as the sender future; otherwise the response head
		// never arrives. `tokio::join!` polls both each time we are polled
		// and registers wakers for both while the head is in flight. Once
		// the head arrives, we hand the driver future to the
		// `DrivenBody` (the inline-driven body wrapper) that backs the response, which polls it in line
		// with the body frames from there on (#1900).
		//
		// A short response (for example an error body sent with
		// `Connection: close`) can complete the connection future in the
		// same poll that delivers the response head. Tracking that here
		// keeps `DrivenBody` from re-polling a completed future, which
		// violates the `Future` contract.
		let send_fut = sender.send_request(req);
		tokio::pin!(send_fut);
		let mut conn_state = ConnState::new(driver);
		let send_result = Self::apply_timeout(
			response_timeout,
			"waiting for the Podman socket to respond",
			futures_util::future::poll_fn(|cx| {
				conn_state.poll(cx);
				send_fut.as_mut().poll(cx)
			}),
		)
		.await;
		match (send_result, conn_state.result()) {
			(Ok(Ok(resp)), Some(Err(e))) => {
				// The head arrived and the connection then failed in the same
				// poll. When the connection ran on its own task, the caller got
				// the response regardless and the body reported an error only
				// if bytes were actually missing; keep that, so a response the
				// daemon finished writing before the socket failed stays usable.
				tracing::debug!(
					"libpod connection closed with an error after the response head: {e}"
				);
				let (parts, body) = resp.into_parts();
				Ok(Response::from_parts(parts, DrivenBody::new(body, None)))
			}
			(Ok(Ok(resp)), _) => {
				let (parts, body) = resp.into_parts();
				Ok(Response::from_parts(
					parts,
					DrivenBody::new(body, conn_state.take_pending()),
				))
			}
			(Ok(Err(e)), _) => Err(PodmanError::Hyper(e)),
			(Err(e), _) => Err(e),
		}
	}

	/// Read the full response body off a streaming connection. The body is
	/// generic so the buffered path
	/// ([`BufferedResponse::read_body`](super::buffered::BufferedResponse::read_body),
	/// body is `Incoming`) and the streaming path (`stream_or_err`, body is
	/// `DrivenBody` (the inline-driven body wrapper) share the size cap and the
	/// timeout handling. The cap
	/// prevents a runaway response from holding more than `MAX_RESPONSE_BYTES`
	/// in memory; it does not bound the duration of a long-lived stream
	/// (those callers keep the body and poll it themselves).
	pub(in crate::connection::libpod::client) async fn read_response_body<B>(
		resp: Response<B>,
		read_timeout: Option<std::time::Duration>,
	) -> Result<(StatusCode, Vec<u8>)>
	where
		B: hyper::body::Body<Data = Bytes, Error = hyper::Error> + Send + Unpin + 'static,
	{
		let status = resp.status();
		let read = Limited::new(resp.into_body(), MAX_RESPONSE_BYTES).collect();
		let collected = Self::apply_timeout(
			read_timeout,
			"reading the response body from the Podman socket",
			read,
		)
		.await?
		.map_err(
			|e: Box<dyn std::error::Error + Send + Sync>| PodmanError::Api {
				status: 0,
				message: format!("reading response body: {e}"),
			},
		)?;
		Ok((status, collected.to_bytes().to_vec()))
	}

	/// For streaming endpoints, return the response on success or parse the
	/// daemon error body on failure.
	pub(in crate::connection::libpod::client) async fn stream_or_err(
		resp: Response<DrivenBody>,
	) -> Result<Response<DrivenBody>> {
		if resp.status().is_success() {
			return Ok(resp);
		}
		let (status, body) = Self::read_response_body(resp, Some(READ_TIMEOUT)).await?;
		Self::check_status(status, &body)?;
		unreachable!("check_status returns Err for a non-success status")
	}
}
