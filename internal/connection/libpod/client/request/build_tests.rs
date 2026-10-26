//! Unit tests for `Client::build_request`: the request target reaches hyper in
//! origin form, and every shape that would put something other than `libpod`
//! in Podman's `split[2]` slot is rejected before the bytes leave the client
//! (#1914).

use super::transport::full;
use super::Client;

// ---------------------------------------------------------------------------
// build_request tests
// ---------------------------------------------------------------------------

#[test]
fn build_request_valid_path() {
	use bytes::Bytes;
	use hyper::Method;
	Client::build_request(Method::GET, "/libpod/_ping", full(Bytes::new()), None).unwrap();
}

#[test]
fn build_request_sets_content_type_when_given() {
	use bytes::Bytes;
	use hyper::Method;
	let req = Client::build_request(
		Method::POST,
		"/libpod/secrets/create",
		full(Bytes::new()),
		Some("application/json"),
	)
	.unwrap();
	assert_eq!(
		req.headers()
			.get(hyper::header::CONTENT_TYPE)
			.and_then(|v| v.to_str().ok()),
		Some("application/json")
	);
}

/// The request target must reach hyper as origin form: no scheme, no authority,
/// just the path and its query string. Writing `http://localhost<path>` instead
/// landed the request line on the wire in absolute form
/// (`POST http://localhost/... HTTP/1.1`), and Podman uses
/// `strings.Split(r.URL.String(), "/")[2] == "libpod"` to decide which
/// handlers to call. For an absolute-form target that slot holds `localhost`,
/// so the shared handlers (Docker compat) answered podup instead of the
/// libpod ones (#1914). hyper parses `scheme`/`authority` straight off the
/// `Uri`, so an empty scheme + empty authority is what guarantees the
/// request target is sent in origin form.
#[test]
fn build_request_writes_origin_form_uri() {
	use bytes::Bytes;
	use hyper::Method;
	let path = "/libpod/build?a=1&b=2";
	let req = Client::build_request(Method::POST, path, full(Bytes::new()), None).unwrap();
	assert_eq!(
		req.uri().scheme(),
		None,
		"scheme must be absent for origin form"
	);
	assert_eq!(
		req.uri().authority(),
		None,
		"authority must be absent for origin form"
	);
	assert_eq!(
		req.uri()
			.path_and_query()
			.map(|p| p.as_str().to_string())
			.as_deref(),
		Some(path),
		"path_and_query must be the input path verbatim, with the query string",
	);
	assert_eq!(
		req.headers()
			.get(hyper::header::HOST)
			.and_then(|v| v.to_str().ok()),
		Some("localhost"),
		"HTTP/1.1 still requires the Host header on an origin-form request",
	);
}

/// A path with no leading slash parses to the authority form
/// (`libpod/_ping` => authority = `libpod/_ping`, path = empty), which puts
/// `libpod/_ping` in Podman's `split[2]` slot and selects neither the libpod
/// nor the Docker handler. Reject it loudly so a caller cannot silently fall
/// back into it (#1914).
#[test]
fn build_request_rejects_path_without_leading_slash() {
	use bytes::Bytes;
	use hyper::Method;
	let err =
		Client::build_request(Method::GET, "libpod/_ping", full(Bytes::new()), None).unwrap_err();
	assert!(err.to_string().contains("invalid API path"), "got: {err}");
}

/// An absolute URI has a scheme and an authority, so the request target on
/// the wire is `http://localhost/libpod/_ping`. Podman's
/// `strings.Split(r.URL.String(), "/")[2]` reads `localhost` from that, not
/// `libpod`, and the shared (Docker) handlers answer. Reject the absolute
/// form before the bytes ever leave the client (#1914).
#[test]
fn build_request_rejects_absolute_uri() {
	use bytes::Bytes;
	use hyper::Method;
	let err = Client::build_request(
		Method::GET,
		"http://localhost/libpod/_ping",
		full(Bytes::new()),
		None,
	)
	.unwrap_err();
	assert!(err.to_string().contains("invalid API path"), "got: {err}");
}

#[test]
fn build_request_rejects_unparseable_path() {
	use bytes::Bytes;
	use hyper::Method;
	// A control character makes the path an invalid `Uri`, which must surface
	// as a structured Api error rather than panicking.
	let err = Client::build_request(
		Method::GET,
		"/libpod/bad\u{7f}path",
		full(Bytes::new()),
		None,
	)
	.unwrap_err();
	assert!(err.to_string().contains("invalid API path"));
}
