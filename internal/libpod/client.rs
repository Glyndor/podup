//! HTTP client for the Podman libpod REST API.
//!
//! Opens a new connection per request over the Podman Unix socket (or named
//! pipe on Windows). Connection-per-request is correct for a CLI tool where
//! API calls are sequential and infrequent.

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::client::conn::http1;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{de::DeserializeOwned, Serialize};

use super::error::PodmanError;

type BoxBody = Full<Bytes>;

pub type Result<T> = std::result::Result<T, PodmanError>;

/// Podman libpod REST API client.
pub struct Client {
	socket_path: String,
}

impl Client {
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
		let mut sender = self.connect().await?;
		sender.send_request(req).await.map_err(PodmanError::Hyper)
	}

	/// Read the full response body into a `Vec<u8>`.
	async fn read_body(resp: Response<Incoming>) -> Result<(StatusCode, Vec<u8>)> {
		let status = resp.status();
		let bytes = resp
			.into_body()
			.collect()
			.await
			.map_err(PodmanError::Hyper)?
			.to_bytes();
		Ok((status, bytes.to_vec()))
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

	// ---------------------------------------------------------------------------
	// Request helpers
	// ---------------------------------------------------------------------------

	/// `GET /libpod/_ping` — returns Ok(()) when Podman is reachable.
	pub async fn ping(&self) -> Result<()> {
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
		let resp = self.send(req).await?;
		if !resp.status().is_success() {
			let (status, body) = Self::read_body(resp).await?;
			return Err(PodmanError::Api {
				status: status.as_u16(),
				message: String::from_utf8_lossy(&body).into_owned(),
			});
		}
		Ok(resp)
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
		let resp = self.send(req).await?;
		if !resp.status().is_success() {
			let (status, body) = Self::read_body(resp).await?;
			return Err(PodmanError::Api {
				status: status.as_u16(),
				message: String::from_utf8_lossy(&body).into_owned(),
			});
		}
		Ok(resp)
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
		let resp = self.send(req).await?;
		if !resp.status().is_success() {
			let (status, body) = Self::read_body(resp).await?;
			return Err(PodmanError::Api {
				status: status.as_u16(),
				message: String::from_utf8_lossy(&body).into_owned(),
			});
		}
		Ok(resp)
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
		let resp = self.send(req).await?;
		if !resp.status().is_success() {
			let (status, body) = Self::read_body(resp).await?;
			return Err(PodmanError::Api {
				status: status.as_u16(),
				message: String::from_utf8_lossy(&body).into_owned(),
			});
		}
		Ok(resp)
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

// ---------------------------------------------------------------------------
// URL encoding helpers
// ---------------------------------------------------------------------------

pub(crate) fn urlencoded(s: &str) -> String {
	let mut out = String::with_capacity(s.len());
	for b in s.bytes() {
		match b {
			b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
				out.push(b as char);
			}
			_ => {
				out.push('%');
				out.push(
					char::from_digit((b >> 4) as u32, 16)
						.unwrap()
						.to_ascii_uppercase(),
				);
				out.push(
					char::from_digit((b & 0xf) as u32, 16)
						.unwrap()
						.to_ascii_uppercase(),
				);
			}
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::urlencoded;

	#[test]
	fn unreserved_chars_pass_through() {
		assert_eq!(urlencoded("abc-XYZ_0.9~"), "abc-XYZ_0.9~");
	}

	#[test]
	fn space_encoded() {
		assert_eq!(urlencoded("hello world"), "hello%20world");
	}

	#[test]
	fn slash_encoded() {
		assert_eq!(urlencoded("a/b"), "a%2Fb");
	}

	#[test]
	fn colon_encoded() {
		assert_eq!(urlencoded("myproj:v1"), "myproj%3Av1");
	}

	#[test]
	fn empty_string() {
		assert_eq!(urlencoded(""), "");
	}

	#[test]
	fn unicode_byte_encoded() {
		// '€' = 0xE2 0x82 0xAC in UTF-8
		assert_eq!(urlencoded("€"), "%E2%82%AC");
	}

	#[test]
	fn container_name_typical() {
		assert_eq!(urlencoded("myproject-web"), "myproject-web");
	}

	#[test]
	fn container_name_with_brackets() {
		assert_eq!(urlencoded("a[b]"), "a%5Bb%5D");
	}
}
