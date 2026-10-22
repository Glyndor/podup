//! Request building for the libpod client: turn a method, path, body, and
//! optional content type into the [`hyper::Request`] the rest of the client
//! sends.
//!
//! Split from `mod.rs` so the request-shape concern (origin-form URI,
//! `Host: localhost`, optional content type) lives on its own.

use hyper::{Method, Request};

use super::{BoxBody, Client, PodmanError, Result};

impl Client {
	/// Build a request with an optional JSON body.
	///
	/// The request target is sent in origin form (`POST /v5.0.0/libpod/build?...
	/// HTTP/1.1`), the same shape `podman --remote` and curl write when they
	/// talk to the Podman socket. Podman decides "is this a libpod request"
	/// by splitting `r.URL.String()` on `/` and reading `split[2]`; an
	/// absolute-form target (`http://localhost/...`) puts `localhost` in
	/// `split[2]` instead of `libpod`, so every shared handler treats the
	/// client as Docker and podup loses the libpod semantics every
	/// compensation in this patch exists to keep stable. Origin form avoids
	/// that because the URL string is just the path. The `Host: localhost`
	/// header stays because HTTP/1.1 requires it.
	///
	/// `path` must start with `/`; paths without a leading slash
	/// (`libpod/_ping`) parse to authority form and absolute URIs
	/// (`http://localhost/libpod/_ping`) put a scheme + authority on the URL
	/// string, both of which put `localhost` in `split[2]` and quietly lose
	/// the same semantics. Reject them with `invalid API path` so neither
	/// shape can slip through.
	pub(in crate::libpod::client) fn build_request(
		method: Method,
		path: &str,
		body: BoxBody,
		content_type: Option<&str>,
	) -> Result<Request<BoxBody>> {
		let uri: hyper::Uri =
			path.parse()
				.map_err(|e: hyper::http::uri::InvalidUri| PodmanError::Api {
					status: 0,
					message: format!("invalid API path '{path}': {e}"),
				})?;
		if uri.scheme().is_some() || uri.authority().is_some() {
			return Err(PodmanError::Api {
				status: 0,
				message: format!(
					"invalid API path '{path}': path must start with '/' and contain no scheme or authority"
				),
			});
		}

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
}
