//! HTTP client for the Podman libpod REST API.
//!
//! Reuses HTTP/1.1 connections to the Podman Unix socket (or named pipe on
//! Windows) across requests through the per-socket pool in
//! [`client::pool`](self). Buffered calls acquire a connection, issue one
//! request, and release it on completion; streaming calls take a dedicated
//! connection for the lifetime of the stream and release it when the body
//! drops. See [`Client`] for the full contract.
//!
//! This file owns the [`Client`] struct, the [`MIN_LIBPOD_API_MAJOR`] floor
//! (read off the source by `tests/podman_floor.rs`), the crate-internal
//! [`Result`] alias, and the `mod` declarations that wire the rest of the
//! client together. The supporting code lives one concern per file:
//!
//! - [`transport`](self::transport): boxed body, timeouts, connection-close
//!   detection, the generic `apply_timeout` wrapper.
//! - [`socket`](self::socket): `socket_error` (unix-only operator hints).
//! - [`path_stat`](self::path_stat): the `X-Docker-Container-Path-Stat` type.
//! - [`request`](self::request): `Client::build_request`.
//! - [`send`](self::send): buffered send, `BufferedResponse`.
//! - [`streaming`](self::streaming): streaming send, body read, error path.
//! - [`status`](self::status): JSON error decoding and version gate.

use std::sync::Arc;

use super::error::PodmanError;

mod conn_state;
mod delete;
mod encode;
mod get;
mod hijack;
mod misc;
mod path_stat;
mod pool;
mod post;
mod put;
mod request;
mod send;
mod socket;
mod status;
mod stream;
mod stream_body;
mod streaming;
mod transport;

pub(crate) use encode::{is_valid_object_name, urlencoded};
pub(crate) use hijack::Hijacked;
use pool::ConnPool;
use stream::SocketStream;
pub use stream_body::DrivenBody;
// Re-exports for `super::X` from siblings and tests. Kept here so the
// `super::full`, `super::READ_TIMEOUT`, `super::meets_minimum`, etc.
// paths continue to resolve at the parent without touching the verb files.
pub(crate) use path_stat::PathStat;
#[cfg(unix)]
pub(crate) use socket::socket_error;
use status::meets_minimum;
use transport::{full, BoxBody, READ_TIMEOUT};

/// Result alias for libpod client calls, fixing the error to [`PodmanError`].
pub type Result<T> = std::result::Result<T, PodmanError>;

/// Podman libpod REST API client.
///
/// Holds an HTTP/1.1 connection pool keyed by socket path. Buffered calls
/// acquire a connection, issue one request, and release the connection on
/// completion; connections that observed an error are dropped instead of
/// returned to the pool. Streaming calls (`get_stream`, `post_json_stream`,
/// `post_empty_stream`, `post_bytes_stream`, `post_stream_body`,
/// `post_json_stream_within`) take a dedicated connection for the lifetime of
/// the stream's response body. The streaming connection lives inside the
/// response body (the inline-driven `DrivenBody`); dropping the
/// body closes the socket, so the [`Client`] does not track streaming
/// connections itself (#1900).
pub struct Client {
	socket_path: String,
	pool: Arc<ConnPool>,
}

/// Lowest libpod API major version podup supports. Podman 5.x reports `5.x.y`;
/// anything below `5.0` lacks SpecGenerator fields podup relies on.
///
/// `tests/podman_floor.rs` reads this declaration out of `mod.rs` as text and
/// asserts the value stays in lockstep with `install.sh` and
/// `debian/control`, so the constant stays in this file even though most of
/// the rest of the module split out.
const MIN_LIBPOD_API_MAJOR: u64 = 5;

impl Drop for Client {
	/// Close every held connection. Idle pooled connections are dropped via
	/// the pool's `close`, which wakes any blocked acquirers with a closed
	/// error. Streaming connections live inside their response body
	/// (`DrivenBody`) and close their sockets when
	/// the body is dropped, so the [`Client`] itself does not need to track
	/// them (#1900).
	fn drop(&mut self) {
		self.pool.close();
	}
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "streaming_close_tests.rs"]
mod streaming_close_tests;
