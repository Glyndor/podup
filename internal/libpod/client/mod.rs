//! HTTP client for the Podman libpod REST API.
//!
//! Opens a new connection per request over the Podman Unix socket (or named
//! pipe on Windows). Connection-per-request is correct for a CLI tool where
//! API calls are sequential and infrequent.

use bytes::Bytes;
use futures_util::Stream;
use http_body_util::{BodyExt, Full, Limited, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::client::conn::http1;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{de::DeserializeOwned, Serialize};

use super::error::PodmanError;

mod encode;
pub(crate) use encode::{is_valid_object_name, urlencoded};

/// The request body every call shares. A boxed body so a fully-buffered
/// `Full<Bytes>` (almost every call) and a lazily-streamed build-context body
/// (the `build` endpoint) travel the same client path. `Unsync` because hyper's
/// `send_request` only requires the body to be `Send`, and the streamed body is
/// not `Sync`.
type BoxBody = http_body_util::combinators::UnsyncBoxBody<Bytes, std::io::Error>;

/// Box a fully-buffered byte payload into [`BoxBody`]. `Full`'s error is
/// `Infallible`, mapped to the unified `io::Error` (which it never produces).
fn full(bytes: Bytes) -> BoxBody {
	Full::new(bytes)
		.map_err(|never| match never {})
		.boxed_unsync()
}

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
	///
	/// `response_timeout` bounds how long we wait for the server to return the
	/// response head. Pass `Some` (the default [`READ_TIMEOUT`]) for ordinary and
	/// streaming calls, where the head arrives promptly — this stops a socket that
	/// accepts the connection but never replies from hanging the CLI indefinitely.
	/// Pass `None` only for endpoints that legitimately block server-side before
	/// the head (e.g. `wait?condition=stopped`), whose callers impose an outer
	/// budget.
	async fn send(
		&self,
		req: Request<BoxBody>,
		response_timeout: Option<std::time::Duration>,
	) -> Result<Response<Incoming>> {
		tracing::debug!("libpod {} {}", req.method(), req.uri().path());
		let mut sender = tokio::time::timeout(CONNECT_TIMEOUT, self.connect())
			.await
			.map_err(|_| PodmanError::Api {
				status: 0,
				message: format!(
					"timed out after {}s connecting to the Podman socket",
					CONNECT_TIMEOUT.as_secs()
				),
			})??;
		let request = sender.send_request(req);
		Self::apply_timeout(
			response_timeout,
			"waiting for the Podman socket to respond",
			request,
		)
		.await?
		.map_err(PodmanError::Hyper)
	}

	/// Read the full response body into a `Vec<u8>`, capped at
	/// [`MAX_RESPONSE_BYTES`] so a rogue or runaway daemon cannot exhaust memory.
	///
	/// `read_timeout` bounds how long we wait for the body. Pass `Some` to apply a
	/// ceiling (the default [`READ_TIMEOUT`] for ordinary buffered calls); pass
	/// `None` for endpoints that legitimately block server-side for an unbounded
	/// duration (e.g. `wait?condition=stopped`, where the caller imposes its own
	/// outer budget).
	async fn read_body(
		resp: Response<Incoming>,
		read_timeout: Option<std::time::Duration>,
	) -> Result<(StatusCode, Vec<u8>)> {
		let status = resp.status();
		let read = Limited::new(resp.into_body(), MAX_RESPONSE_BYTES).collect();
		let collected = Self::apply_timeout(
			read_timeout,
			"reading the response body from the Podman socket",
			read,
		)
		.await?
		.map_err(|e| PodmanError::Api {
			status: 0,
			message: format!("reading response body: {e}"),
		})?;
		Ok((status, collected.to_bytes().to_vec()))
	}

	/// Await `fut`, optionally bounded by `timeout`.
	///
	/// With `Some(limit)` a stalled future is aborted once `limit` elapses, yielding
	/// a timeout [`PodmanError`] whose message names `phase` (what we were waiting
	/// on); with `None` it is awaited uncapped, for endpoints that legitimately
	/// block server-side (the caller supplies its own outer budget). Shared by the
	/// response-head wait ([`send`](Self::send)) and the body read
	/// ([`read_body`](Self::read_body)); split out so the policy is testable without
	/// a live socket.
	async fn apply_timeout<F, T>(
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
	/// the body and surface it through [`check_status`](Self::check_status) so the caller gets the
	/// parsed Podman error message rather than the raw JSON body.
	async fn stream_or_err(resp: Response<Incoming>) -> Result<Response<Incoming>> {
		if resp.status().is_success() {
			return Ok(resp);
		}
		let (status, body) = Self::read_body(resp, Some(READ_TIMEOUT)).await?;
		Self::check_status(status, &body)?;
		unreachable!("check_status returns Err for a non-success status")
	}

	// ---------------------------------------------------------------------------
	// Request helpers
	// ---------------------------------------------------------------------------

	/// `GET /libpod/_ping` — returns Ok(()) when Podman is reachable *and* speaks
	/// a libpod API version podup supports.
	///
	/// Podman answers `_ping` with a `Libpod-API-Version` response header. We read
	/// it here, while the call is already cheap, and reject a server below the
	/// `MIN_LIBPOD_API_MAJOR.0` floor with a clear
	/// `PodmanError::IncompatibleApiVersion` rather than letting a later
	/// SpecGenerator or libpod-native call fail with an obscure 4xx.
	pub async fn ping(&self) -> Result<()> {
		// Deliberately omits the version prefix: `_ping` is version-independent.
		let req = Self::build_request(Method::GET, "/libpod/_ping", full(Bytes::new()), None)?;
		let resp = self.send(req, Some(READ_TIMEOUT)).await?;
		// Read the version header before the body is consumed below.
		let reported = resp
			.headers()
			.get("Libpod-API-Version")
			.and_then(|v| v.to_str().ok())
			.unwrap_or_default()
			.to_owned();
		let (status, body) = Self::read_body(resp, Some(READ_TIMEOUT)).await?;
		Self::check_status(status, &body)?;
		if !meets_minimum(&reported) {
			return Err(PodmanError::IncompatibleApiVersion { reported });
		}
		Ok(())
	}

	/// `GET` → deserialize JSON response.
	pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
		let req = Self::build_request(Method::GET, path, full(Bytes::new()), None)?;
		let resp = self.send(req, Some(READ_TIMEOUT)).await?;
		let (status, body) = Self::read_body(resp, Some(READ_TIMEOUT)).await?;
		Self::check_status(status, &body)?;
		serde_json::from_slice(&body).map_err(PodmanError::Json)
	}

	/// `GET` → return raw `Response<Incoming>` for streaming.
	pub async fn get_stream(&self, path: &str) -> Result<Response<Incoming>> {
		let req = Self::build_request(Method::GET, path, full(Bytes::new()), None)?;
		Self::stream_or_err(self.send(req, Some(READ_TIMEOUT)).await?).await
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
			full(Bytes::from(json)),
			Some("application/json"),
		)?;
		let resp = self.send(req, Some(READ_TIMEOUT)).await?;
		let (status, body) = Self::read_body(resp, Some(READ_TIMEOUT)).await?;
		Self::check_status(status, &body)?;
		serde_json::from_slice(&body).map_err(PodmanError::Json)
	}

	/// `POST` with JSON body → ignore response body (expect 2xx).
	pub async fn post_json_ok<B: Serialize>(&self, path: &str, body: &B) -> Result<()> {
		let json = serde_json::to_vec(body).map_err(PodmanError::Json)?;
		let req = Self::build_request(
			Method::POST,
			path,
			full(Bytes::from(json)),
			Some("application/json"),
		)?;
		let resp = self.send(req, Some(READ_TIMEOUT)).await?;
		let (status, body) = Self::read_body(resp, Some(READ_TIMEOUT)).await?;
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
			full(Bytes::from(json)),
			Some("application/json"),
		)?;
		Self::stream_or_err(self.send(req, Some(READ_TIMEOUT)).await?).await
	}

	/// `POST` with empty body → ignore response body (expect 2xx or 304).
	pub async fn post_empty_ok(&self, path: &str) -> Result<()> {
		let req = Self::build_request(Method::POST, path, full(Bytes::new()), None)?;
		let resp = self.send(req, Some(READ_TIMEOUT)).await?;
		let (status, body) = Self::read_body(resp, Some(READ_TIMEOUT)).await?;
		// 304 Not Modified is fine for idempotent ops
		if status == StatusCode::NOT_MODIFIED {
			return Ok(());
		}
		Self::check_status(status, &body)
	}

	/// `POST` with empty body → ignore response body (expect 2xx or 304), bounded
	/// by a caller-chosen deadline rather than the default `READ_TIMEOUT`.
	///
	/// `deadline` of `Some` caps both the response-head wait and the body read so a
	/// `stop` on a container that is slow to die (or a wedged libpod call) returns a
	/// timeout error after the grace window instead of pinning the CLI for the full
	/// `READ_TIMEOUT`; `None` leaves it uncapped (docker `stop -t -1` parity). The
	/// caller decides whether a resulting `PodmanError::is_timeout` warrants a
	/// client-side `SIGKILL`/force-remove escalation.
	pub async fn post_empty_ok_within(
		&self,
		path: &str,
		deadline: Option<std::time::Duration>,
	) -> Result<()> {
		let req = Self::build_request(Method::POST, path, full(Bytes::new()), None)?;
		let resp = self.send(req, deadline).await?;
		let (status, body) = Self::read_body(resp, deadline).await?;
		// 304 Not Modified is fine for idempotent ops
		if status == StatusCode::NOT_MODIFIED {
			return Ok(());
		}
		Self::check_status(status, &body)
	}

	/// `POST` with JSON body → return raw `Response<Incoming>` for streaming,
	/// bounding the wait for the response head by `head_timeout` instead of the
	/// default `READ_TIMEOUT`.
	///
	/// `exec`-start uses this with a short, exec-specific ceiling: a healthy engine
	/// returns the start head (the hijack, or a prompt error) almost immediately, so
	/// a long wait means the launch is wedged — e.g. a nonexistent target user the
	/// server stalls resolving. Bounding the head lets the caller fail fast with a
	/// clear, exec-specific message rather than pinning the CLI for the full
	/// `READ_TIMEOUT` and then reporting a misleading socket-timeout. The streamed
	/// body is left unbounded (`head_timeout` covers only the head), so a legitimate
	/// long-running exec still streams normally.
	pub async fn post_json_stream_within<B: Serialize>(
		&self,
		path: &str,
		body: &B,
		head_timeout: Option<std::time::Duration>,
	) -> Result<Response<Incoming>> {
		let json = serde_json::to_vec(body).map_err(PodmanError::Json)?;
		let req = Self::build_request(
			Method::POST,
			path,
			full(Bytes::from(json)),
			Some("application/json"),
		)?;
		Self::stream_or_err(self.send(req, head_timeout).await?).await
	}

	/// `POST` with empty body → return raw `Response<Incoming>` for streaming.
	pub async fn post_empty_stream(&self, path: &str) -> Result<Response<Incoming>> {
		let req = Self::build_request(Method::POST, path, full(Bytes::new()), None)?;
		Self::stream_or_err(self.send(req, Some(READ_TIMEOUT)).await?).await
	}

	/// `POST` with empty body → deserialize JSON response.
	pub async fn post_empty_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
		let req = Self::build_request(Method::POST, path, full(Bytes::new()), None)?;
		let resp = self.send(req, Some(READ_TIMEOUT)).await?;
		let (status, body) = Self::read_body(resp, Some(READ_TIMEOUT)).await?;
		Self::check_status(status, &body)?;
		serde_json::from_slice(&body).map_err(PodmanError::Json)
	}

	/// `POST` with empty body → deserialize JSON response, with **no** read-timeout
	/// ceiling on the response body.
	///
	/// For blocking endpoints that legitimately hold the connection open for an
	/// arbitrary, server-side duration — notably `containers/{name}/wait`, which
	/// does not respond until the container reaches the requested condition. The
	/// default `READ_TIMEOUT` would otherwise abort the call after 120 s and
	/// surface a spurious timeout instead of the real exit code, so callers of
	/// this method must impose their own outer budget (e.g. a
	/// [`tokio::time::timeout`]) to stay bounded.
	pub async fn post_empty_json_unbounded<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
		let req = Self::build_request(Method::POST, path, full(Bytes::new()), None)?;
		let resp = self.send(req, None).await?;
		let (status, body) = Self::read_body(resp, None).await?;
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
		let req = Self::build_request(Method::POST, path, full(bytes), Some(content_type))?;
		Self::stream_or_err(self.send(req, Some(READ_TIMEOUT)).await?).await
	}

	/// `POST` with a **streamed** body → return raw `Response<Incoming>` for
	/// streaming.
	///
	/// The body is produced lazily from `chunks` rather than buffered whole, so a
	/// large upload (a multi-gigabyte build-context tar) never inflates the
	/// process's RSS. Each item is an `http_body`-style frame or a terminal
	/// `io::Error` that aborts the request.
	pub async fn post_stream_body<S>(
		&self,
		path: &str,
		chunks: S,
		content_type: &str,
	) -> Result<Response<Incoming>>
	where
		S: Stream<Item = std::result::Result<Frame<Bytes>, std::io::Error>> + Send + 'static,
	{
		let body = StreamBody::new(chunks).boxed_unsync();
		let req = Self::build_request(Method::POST, path, body, Some(content_type))?;
		Self::stream_or_err(self.send(req, Some(READ_TIMEOUT)).await?).await
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
		let req = Self::build_request(Method::POST, path, full(bytes), Some(content_type))?;
		let resp = self.send(req, Some(READ_TIMEOUT)).await?;
		let (status, body) = Self::read_body(resp, Some(READ_TIMEOUT)).await?;
		Self::check_status(status, &body)?;
		serde_json::from_slice(&body).map_err(PodmanError::Json)
	}

	/// `PUT` with raw bytes body → expect 2xx.
	pub async fn put_bytes_ok(&self, path: &str, bytes: Bytes, content_type: &str) -> Result<()> {
		let req = Self::build_request(Method::PUT, path, full(bytes), Some(content_type))?;
		let resp = self.send(req, Some(READ_TIMEOUT)).await?;
		let (status, body) = Self::read_body(resp, Some(READ_TIMEOUT)).await?;
		Self::check_status(status, &body)
	}

	/// `HEAD` a container-archive path, returning `Some(is_dir)` when it exists or
	/// `None` on 404. Reads the `X-Docker-Container-Path-Stat` header (base64 JSON
	/// carrying a Go file `mode`); the directory bit is `os.ModeDir` (`1 << 31`).
	/// Lets `cp` tell an existing destination directory (copy into it) from a
	/// target name (rename on copy), matching `docker cp`.
	pub async fn head_path_is_dir(&self, path: &str) -> Result<Option<bool>> {
		use base64::Engine as _;

		let req = Self::build_request(Method::HEAD, path, full(Bytes::new()), None)?;
		let resp = self.send(req, Some(READ_TIMEOUT)).await?;
		let status = resp.status();
		if status == StatusCode::NOT_FOUND {
			return Ok(None);
		}
		let stat = resp
			.headers()
			.get("X-Docker-Container-Path-Stat")
			.and_then(|v| v.to_str().ok())
			.map(str::to_string);
		let (status, body) = Self::read_body(resp, Some(READ_TIMEOUT)).await?;
		if status == StatusCode::NOT_FOUND {
			return Ok(None);
		}
		Self::check_status(status, &body)?;
		let Some(stat) = stat else {
			return Ok(Some(false));
		};
		let json = base64::engine::general_purpose::STANDARD
			.decode(stat.as_bytes())
			.map_err(|e| PodmanError::Api {
				status: 0,
				message: format!("malformed container path stat: {e}"),
			})?;
		#[derive(serde::Deserialize)]
		struct Stat {
			mode: u64,
		}
		let parsed: Stat = serde_json::from_slice(&json).map_err(PodmanError::Json)?;
		// Go's os.ModeDir is the high bit of the 32-bit FileMode.
		Ok(Some(parsed.mode & (1 << 31) != 0))
	}

	/// `DELETE` → `Ok(true)` if the resource existed and was removed, `Ok(false)`
	/// on a 404 (nothing to delete). Lets a caller tell a real deletion from a
	/// no-op, so it can avoid reporting a phantom "removed" for a container that
	/// never existed.
	pub async fn delete_existed(&self, path: &str) -> Result<bool> {
		let req = Self::build_request(Method::DELETE, path, full(Bytes::new()), None)?;
		let resp = self.send(req, Some(READ_TIMEOUT)).await?;
		let (status, body) = Self::read_body(resp, Some(READ_TIMEOUT)).await?;
		if status == StatusCode::NOT_FOUND {
			return Ok(false);
		}
		Self::check_status(status, &body)?;
		Ok(true)
	}

	/// `DELETE` → ignore response body (expect 2xx or 404). A 404 is an
	/// idempotent no-op; see [`Self::delete_existed`] when the distinction matters.
	pub async fn delete_ok(&self, path: &str) -> Result<()> {
		self.delete_existed(path).await.map(|_| ())
	}
}

/// Lowest libpod API major version podup supports. Podman 5.x reports `5.x.y`;
/// anything below `5.0` lacks SpecGenerator fields podup relies on.
const MIN_LIBPOD_API_MAJOR: u64 = 5;

/// Whether a `Libpod-API-Version` string (e.g. `"5.0.0"`, `"4.9.3"`) meets the
/// [`MIN_LIBPOD_API_MAJOR`].0 floor.
///
/// Pure and total so it is unit-testable in isolation. Only the major component
/// gates: any `5.x.y` (or higher major) passes; `4.x.y` is rejected. An empty or
/// malformed string — a server that sent no header, or a value we cannot parse —
/// is treated as *not* meeting the minimum, so we fail closed rather than assume
/// a compatible server.
fn meets_minimum(version: &str) -> bool {
	version
		.trim()
		.trim_start_matches('v')
		.split('.')
		.next()
		.and_then(|major| major.parse::<u64>().ok())
		.is_some_and(|major| major >= MIN_LIBPOD_API_MAJOR)
}

#[cfg(test)]
mod tests;
