//! HTTP client for the Podman libpod REST API.
//!
//! Opens a new connection per request over the Podman Unix socket (or named
//! pipe on Windows). Connection-per-request is correct for a CLI tool where
//! API calls are sequential and infrequent.

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::client::conn::http1;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{de::DeserializeOwned, Serialize};

use super::error::PodmanError;

mod encode;
pub(crate) use encode::urlencoded;

type BoxBody = Full<Bytes>;

/// Upper bound on a buffered (non-streaming) response body. Caps memory use
/// when the daemon returns an oversized or runaway response.
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

/// Ceiling on establishing the socket connection and HTTP handshake. Bounds the
/// wait when the Podman socket is absent, busy, or unresponsive. This times the
/// connect only — it does not limit the duration of a streaming response body.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Ceiling on reading a *buffered* (non-streaming) response body. Without it a
/// daemon that accepts the request, sends headers, then stalls would hang the
/// CLI forever. Streaming helpers (logs, attach, archive) are deliberately not
/// bounded by this — they are long-lived by design.
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Result alias for libpod client calls, fixing the error to [`PodmanError`].
pub type Result<T> = std::result::Result<T, PodmanError>;

/// Podman libpod REST API client.
pub struct Client {
	socket_path: String,
}

impl Client {
	/// Create a client bound to the given Podman socket path (or named pipe).
	pub fn new(socket_path: impl Into<String>) -> Self {
		Self {
			socket_path: socket_path.into(),
		}
	}

	/// Open a new HTTP/1.1 sender over the platform socket.
	async fn connect(&self) -> Result<http1::SendRequest<BoxBody>> {
		#[cfg(unix)]
		{
			let stream = tokio::net::UnixStream::connect(&self.socket_path).await?;
			let io = TokioIo::new(stream);
			let (sender, conn) = http1::handshake(io).await?;
			tokio::spawn(async move {
				let _ = conn.await;
			});
			Ok(sender)
		}

		#[cfg(windows)]
		{
			// Named pipes may be momentarily busy; retry a few times.
			let pipe = {
				let mut last_err = None;
				let mut result = None;
				for _ in 0..20 {
					match tokio::net::windows::named_pipe::ClientOptions::new()
						.open(&self.socket_path)
					{
						Ok(p) => {
							result = Some(p);
							break;
						}
						Err(e) if e.raw_os_error() == Some(231) => {
							last_err = Some(e);
							tokio::time::sleep(std::time::Duration::from_millis(50)).await;
						}
						Err(e) => return Err(PodmanError::Connect(e)),
					}
				}
				result.ok_or_else(|| {
					PodmanError::Connect(last_err.unwrap_or_else(|| {
						std::io::Error::new(
							std::io::ErrorKind::TimedOut,
							"named pipe busy after 20 retries",
						)
					}))
				})?
			};
			let io = TokioIo::new(pipe);
			let (sender, conn) = http1::handshake(io).await?;
			tokio::spawn(async move {
				let _ = conn.await;
			});
			Ok(sender)
		}
	}

	/// Build a request with an optional JSON body.
	fn build_request(
		method: Method,
		path: &str,
		body: BoxBody,
		content_type: Option<&str>,
	) -> Result<Request<BoxBody>> {
		let uri: hyper::Uri = format!("http://localhost{path}").parse().map_err(
			|e: hyper::http::uri::InvalidUri| PodmanError::Api {
				status: 0,
				message: format!("invalid API path '{path}': {e}"),
			},
		)?;

		let mut builder = Request::builder()
			.method(method)
			.uri(uri)
			.header(hyper::header::HOST, "localhost");

		if let Some(ct) = content_type {
			builder = builder.header(hyper::header::CONTENT_TYPE, ct);
		}

		builder.body(body).map_err(|e| PodmanError::Api {
			status: 0,
			message: e.to_string(),
		})
	}

	/// Send a request and return the raw response.
	async fn send(&self, req: Request<BoxBody>) -> Result<Response<Incoming>> {
		let mut sender = tokio::time::timeout(CONNECT_TIMEOUT, self.connect())
			.await
			.map_err(|_| PodmanError::Api {
				status: 0,
				message: format!(
					"timed out after {}s connecting to the Podman socket",
					CONNECT_TIMEOUT.as_secs()
				),
			})??;
		sender.send_request(req).await.map_err(PodmanError::Hyper)
	}

	/// Read the full response body into a `Vec<u8>`, capped at
	/// [`MAX_RESPONSE_BYTES`] so a rogue or runaway daemon cannot exhaust memory.
	async fn read_body(resp: Response<Incoming>) -> Result<(StatusCode, Vec<u8>)> {
		let status = resp.status();
		let collected = tokio::time::timeout(
			READ_TIMEOUT,
			Limited::new(resp.into_body(), MAX_RESPONSE_BYTES).collect(),
		)
		.await
		.map_err(|_| PodmanError::Api {
			status: 0,
			message: format!(
				"timed out after {}s reading the response body from the Podman socket",
				READ_TIMEOUT.as_secs()
			),
		})?
		.map_err(|e| PodmanError::Api {
			status: 0,
			message: format!("reading response body: {e}"),
		})?;
		Ok((status, collected.to_bytes().to_vec()))
	}

	/// Check status code; on error parse the Podman error message.
	fn check_status(status: StatusCode, body: &[u8]) -> Result<()> {
		if status.is_success() {
			return Ok(());
		}

		#[derive(serde::Deserialize)]
		struct ApiError {
			cause: Option<String>,
			message: Option<String>,
		}

		let msg = if let Ok(e) = serde_json::from_slice::<ApiError>(body) {
			e.message
				.or(e.cause)
				.unwrap_or_else(|| String::from_utf8_lossy(body).into_owned())
		} else {
			String::from_utf8_lossy(body).into_owned()
		};

		Err(PodmanError::Api {
			status: status.as_u16(),
			message: msg,
		})
	}

	/// For streaming endpoints: return the response on success, otherwise read
	/// the body and surface it through [`check_status`] so the caller gets the
	/// parsed Podman error message rather than the raw JSON body.
	async fn stream_or_err(resp: Response<Incoming>) -> Result<Response<Incoming>> {
		if resp.status().is_success() {
			return Ok(resp);
		}
		let (status, body) = Self::read_body(resp).await?;
		Self::check_status(status, &body)?;
		unreachable!("check_status returns Err for a non-success status")
	}

	// ---------------------------------------------------------------------------
	// Request helpers
	// ---------------------------------------------------------------------------

	/// `GET /libpod/_ping` — returns Ok(()) when Podman is reachable.
	pub async fn ping(&self) -> Result<()> {
		// Deliberately omits the version prefix: `_ping` is version-independent.
		let req = Self::build_request(Method::GET, "/libpod/_ping", Full::new(Bytes::new()), None)?;
		let resp = self.send(req).await?;
		let (status, body) = Self::read_body(resp).await?;
		Self::check_status(status, &body)
	}

	/// `GET` → deserialize JSON response.
	pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
		let req = Self::build_request(Method::GET, path, Full::new(Bytes::new()), None)?;
		let resp = self.send(req).await?;
		let (status, body) = Self::read_body(resp).await?;
		Self::check_status(status, &body)?;
		serde_json::from_slice(&body).map_err(PodmanError::Json)
	}

	/// `GET` → return raw `Response<Incoming>` for streaming.
	pub async fn get_stream(&self, path: &str) -> Result<Response<Incoming>> {
		let req = Self::build_request(Method::GET, path, Full::new(Bytes::new()), None)?;
		Self::stream_or_err(self.send(req).await?).await
	}

	/// `POST` with JSON body → deserialize JSON response.
	pub async fn post_json<B: Serialize, T: DeserializeOwned>(
		&self,
		path: &str,
		body: &B,
	) -> Result<T> {
		let json = serde_json::to_vec(body).map_err(PodmanError::Json)?;
		let req = Self::build_request(
			Method::POST,
			path,
			Full::new(Bytes::from(json)),
			Some("application/json"),
		)?;
		let resp = self.send(req).await?;
		let (status, body) = Self::read_body(resp).await?;
		Self::check_status(status, &body)?;
		serde_json::from_slice(&body).map_err(PodmanError::Json)
	}

	/// `POST` with JSON body → ignore response body (expect 2xx).
	pub async fn post_json_ok<B: Serialize>(&self, path: &str, body: &B) -> Result<()> {
		let json = serde_json::to_vec(body).map_err(PodmanError::Json)?;
		let req = Self::build_request(
			Method::POST,
			path,
			Full::new(Bytes::from(json)),
			Some("application/json"),
		)?;
		let resp = self.send(req).await?;
		let (status, body) = Self::read_body(resp).await?;
		Self::check_status(status, &body)
	}

	/// `POST` with JSON body → return raw `Response<Incoming>` for streaming.
	pub async fn post_json_stream<B: Serialize>(
		&self,
		path: &str,
		body: &B,
	) -> Result<Response<Incoming>> {
		let json = serde_json::to_vec(body).map_err(PodmanError::Json)?;
		let req = Self::build_request(
			Method::POST,
			path,
			Full::new(Bytes::from(json)),
			Some("application/json"),
		)?;
		Self::stream_or_err(self.send(req).await?).await
	}

	/// `POST` with empty body → ignore response body (expect 2xx or 304).
	pub async fn post_empty_ok(&self, path: &str) -> Result<()> {
		let req = Self::build_request(Method::POST, path, Full::new(Bytes::new()), None)?;
		let resp = self.send(req).await?;
		let (status, body) = Self::read_body(resp).await?;
		// 304 Not Modified is fine for idempotent ops
		if status == StatusCode::NOT_MODIFIED {
			return Ok(());
		}
		Self::check_status(status, &body)
	}

	/// `POST` with empty body → return raw `Response<Incoming>` for streaming.
	pub async fn post_empty_stream(&self, path: &str) -> Result<Response<Incoming>> {
		let req = Self::build_request(Method::POST, path, Full::new(Bytes::new()), None)?;
		Self::stream_or_err(self.send(req).await?).await
	}

	/// `POST` with empty body → deserialize JSON response.
	pub async fn post_empty_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
		let req = Self::build_request(Method::POST, path, Full::new(Bytes::new()), None)?;
		let resp = self.send(req).await?;
		let (status, body) = Self::read_body(resp).await?;
		Self::check_status(status, &body)?;
		serde_json::from_slice(&body).map_err(PodmanError::Json)
	}

	/// `POST` with raw bytes body → return raw `Response<Incoming>` for streaming.
	pub async fn post_bytes_stream(
		&self,
		path: &str,
		bytes: Bytes,
		content_type: &str,
	) -> Result<Response<Incoming>> {
		let req = Self::build_request(Method::POST, path, Full::new(bytes), Some(content_type))?;
		Self::stream_or_err(self.send(req).await?).await
	}

	/// `POST` with a raw-bytes body → deserialize JSON response.
	///
	/// Used by endpoints that take a binary payload rather than a JSON object —
	/// e.g. `secrets/create`, whose body is the raw secret data and whose
	/// response is `{"ID": "..."}`.
	pub async fn post_bytes_json<T: DeserializeOwned>(
		&self,
		path: &str,
		bytes: Bytes,
		content_type: &str,
	) -> Result<T> {
		let req = Self::build_request(Method::POST, path, Full::new(bytes), Some(content_type))?;
		let resp = self.send(req).await?;
		let (status, body) = Self::read_body(resp).await?;
		Self::check_status(status, &body)?;
		serde_json::from_slice(&body).map_err(PodmanError::Json)
	}

	/// `PUT` with raw bytes body → expect 2xx.
	pub async fn put_bytes_ok(&self, path: &str, bytes: Bytes, content_type: &str) -> Result<()> {
		let req = Self::build_request(Method::PUT, path, Full::new(bytes), Some(content_type))?;
		let resp = self.send(req).await?;
		let (status, body) = Self::read_body(resp).await?;
		Self::check_status(status, &body)
	}

	/// `DELETE` → ignore response body (expect 2xx or 404).
	pub async fn delete_ok(&self, path: &str) -> Result<()> {
		let req = Self::build_request(Method::DELETE, path, Full::new(Bytes::new()), None)?;
		let resp = self.send(req).await?;
		let (status, body) = Self::read_body(resp).await?;
		if status == StatusCode::NOT_FOUND {
			return Ok(());
		}
		Self::check_status(status, &body)
	}
}

#[cfg(test)]
mod tests {
	use hyper::StatusCode;

	use super::Client;

	// ---------------------------------------------------------------------------
	// check_status tests
	// ---------------------------------------------------------------------------

	#[test]
	fn check_status_ok_on_200() {
		Client::check_status(StatusCode::OK, b"").unwrap();
	}

	#[test]
	fn check_status_ok_on_201() {
		Client::check_status(StatusCode::CREATED, b"").unwrap();
	}

	#[test]
	fn check_status_error_on_404() {
		let err = Client::check_status(StatusCode::NOT_FOUND, b"not found").unwrap_err();
		assert!(err.is_status(404));
		assert!(err.to_string().contains("not found"));
	}

	#[test]
	fn check_status_parses_podman_json_error() {
		let body = br#"{"message":"container not found","cause":"no such container"}"#;
		let err = Client::check_status(StatusCode::NOT_FOUND, body).unwrap_err();
		assert!(err.is_status(404));
		assert!(err.to_string().contains("container not found"));
	}

	#[test]
	fn check_status_falls_back_to_cause_when_no_message() {
		let body = br#"{"cause":"volume in use"}"#;
		let err = Client::check_status(StatusCode::CONFLICT, body).unwrap_err();
		assert!(err.is_status(409));
		assert!(err.to_string().contains("volume in use"));
	}

	#[test]
	fn check_status_falls_back_to_raw_body_on_non_json() {
		let err = Client::check_status(StatusCode::INTERNAL_SERVER_ERROR, b"plain text error")
			.unwrap_err();
		assert!(err.is_status(500));
		assert!(err.to_string().contains("plain text error"));
	}

	// ---------------------------------------------------------------------------
	// build_request tests
	// ---------------------------------------------------------------------------

	#[test]
	fn build_request_valid_path() {
		use bytes::Bytes;
		use http_body_util::Full;
		use hyper::Method;
		Client::build_request(Method::GET, "/libpod/_ping", Full::new(Bytes::new()), None).unwrap();
	}

	#[test]
	fn client_new_stores_socket_path() {
		let c = Client::new("/run/user/1000/podman/podman.sock");
		drop(c); // just verify it constructs
	}
}
