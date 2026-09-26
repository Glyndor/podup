use hyper::StatusCode;

use super::{meets_minimum, Client};

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
	let err =
		Client::check_status(StatusCode::INTERNAL_SERVER_ERROR, b"plain text error").unwrap_err();
	assert!(err.is_status(500));
	assert!(err.to_string().contains("plain text error"));
}

// ---------------------------------------------------------------------------
// parse_error_message tests
// ---------------------------------------------------------------------------

#[test]
fn parse_error_message_prefers_message_field() {
	// Podman's libpod JSON error body carries `message` (operator-facing)
	// and `cause` (lower-level chain). `message` is the one to surface
	// because it is the human-readable reason; `cause` is the wrapped
	// driver detail.
	let body = br#"{"message":"namespace \"evil\" not recognised","cause":"ParseNamespace"}"#;
	let msg = Client::parse_error_message(body);
	assert!(msg.contains("namespace"), "got: {msg}");
	assert!(!msg.contains("ParseNamespace"), "got: {msg}");
}

#[test]
fn parse_error_message_falls_back_to_cause() {
	// Some endpoints populate only `cause`. Falling back keeps the
	// operator looking at libpod's own wording rather than an empty
	// placeholder.
	let body = br#"{"cause":"internal: cgroup mount not found"}"#;
	let msg = Client::parse_error_message(body);
	assert!(msg.contains("cgroup mount"), "got: {msg}");
}

#[test]
fn parse_error_message_uses_raw_body_when_not_json() {
	// A proxy or a 502 from a fronting process can return plain text.
	// The raw body is the only signal then, so it goes through verbatim
	// rather than being dropped to an empty string.
	let body = b"upstream connect error: connection refused";
	let msg = Client::parse_error_message(body);
	assert!(msg.contains("connection refused"), "got: {msg}");
}

#[test]
fn parse_error_message_uses_raw_body_when_json_has_no_message() {
	// An empty `{}` body is JSON but carries no signal; fall through to
	// the raw body so the operator sees at least the byte content.
	let body = b"{}";
	let msg = Client::parse_error_message(body);
	assert!(!msg.is_empty(), "got: {msg}");
}

// ---------------------------------------------------------------------------
// build_request tests
// ---------------------------------------------------------------------------

#[test]
fn build_request_valid_path() {
	use bytes::Bytes;
	use hyper::Method;
	Client::build_request(
		Method::GET,
		"/libpod/_ping",
		super::full(Bytes::new()),
		None,
	)
	.unwrap();
}

#[test]
fn build_request_sets_content_type_when_given() {
	use bytes::Bytes;
	use hyper::Method;
	let req = Client::build_request(
		Method::POST,
		"/libpod/secrets/create",
		super::full(Bytes::new()),
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
	let req = Client::build_request(Method::POST, path, super::full(Bytes::new()), None).unwrap();
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
	let err = Client::build_request(Method::GET, "libpod/_ping", super::full(Bytes::new()), None)
		.unwrap_err();
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
		super::full(Bytes::new()),
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
		super::full(Bytes::new()),
		None,
	)
	.unwrap_err();
	assert!(err.to_string().contains("invalid API path"));
}

#[test]
fn client_new_stores_socket_path() {
	let c = Client::new("/run/user/1000/podman/podman.sock");
	drop(c); // just verify it constructs
}

// ---------------------------------------------------------------------------
// timeout policy tests
// ---------------------------------------------------------------------------

/// A bounded wait aborts a future that outlives the limit and names the phase
/// in the message, the guard that stops a stalled or silent socket (whether
/// waiting on the response head or reading a buffered body) from hanging the
/// CLI. A never-resolving future stands in for the silent-socket attack.
#[tokio::test]
async fn apply_timeout_some_aborts_and_names_phase() {
	let never: std::future::Pending<u8> = std::future::pending();
	let d = std::time::Duration::from_millis(10);
	let msg = Client::apply_timeout(Some(d), "phase-marker", never)
		.await
		.unwrap_err()
		.to_string();
	assert!(msg.contains("timed out") && msg.contains("phase-marker"));
}

/// With `None` the future is awaited uncapped (the `wait?condition=stopped`
/// path, bounded only by the caller's own outer budget).
#[tokio::test]
async fn apply_timeout_none_awaits_uncapped() {
	let value = Client::apply_timeout(None, "phase-marker", async { 42u8 })
		.await
		.unwrap();
	assert_eq!(value, 42);
}

/// The version gate accepts Podman 5.x (and any higher major) and rejects
/// anything older, so an incompatible server is caught at ping time.
#[test]
fn meets_minimum_accepts_5_and_above_rejects_older() {
	assert!(meets_minimum("5.0.0"));
	assert!(meets_minimum("5.4.2"));
	assert!(meets_minimum("6.0.0"));
	// A leading `v` (some libpod builds report `v5.0.0`) is tolerated.
	assert!(meets_minimum("v5.0.0"));
	assert!(!meets_minimum("v4.9.3"));
	assert!(!meets_minimum("4.9.3"));
	assert!(!meets_minimum("4.0.0"));
	assert!(!meets_minimum("3.4.4"));
}

/// A missing or malformed `Libpod-API-Version` fails closed: we never assume a
/// compatible server from an unparseable value.
#[test]
fn meets_minimum_handles_malformed_and_empty() {
	assert!(!meets_minimum(""));
	assert!(!meets_minimum("   "));
	assert!(!meets_minimum("not-a-version"));
	assert!(!meets_minimum(".5"));
	// Leading/trailing whitespace around a valid version is tolerated.
	assert!(meets_minimum(" 5.0.0 "));
}

/// #1097: does a buffered PUT body keep its exact size hint after boxing?
///
/// This is the crux of the leading hypothesis for `cp` into a container failing
/// on Podman 6. hyper sets `Content-Length` when a body reports an exact size
/// and falls back to `Transfer-Encoding: chunked` when it does not. If boxing
/// erased the hint, every `PUT /containers/{id}/archive` would go out chunked,
/// and a server that expects a length would close the connection mid-body,
/// which is exactly the `IncompleteMessage` the lane reports.
///
/// Asserting it here means the hypothesis is settled locally instead of costing
/// a lane round trip, whichever way it falls.
#[test]
fn a_buffered_put_body_reports_an_exact_size() {
	use hyper::body::Body as _;

	let payload = bytes::Bytes::from_static(b"hello world");
	let body = super::full(payload.clone());
	assert_eq!(
		body.size_hint().exact(),
		Some(payload.len() as u64),
		"a boxed Full body must keep its exact size, or hyper sends it chunked"
	);
}

/// The operator's report (#1146): `podman socket connection error: No such
/// file or directory (os error 2)`: no path, no way to tell "it is not there"
/// from "I cannot open it", nothing to act on. Everything needed was already in
/// hand one call earlier.
#[cfg(unix)]
#[test]
fn a_missing_socket_names_the_path_and_the_fix() {
	let e = super::socket_error(
		"/run/user/1000/podman/podman.sock",
		std::io::Error::from(std::io::ErrorKind::NotFound),
	);
	let msg = e.to_string();
	assert!(msg.contains("/run/user/1000/podman/podman.sock"), "{msg}");
	assert!(
		msg.contains("systemctl --user enable --now podman.socket"),
		"{msg}"
	);
	assert!(msg.contains("XDG_RUNTIME_DIR"), "{msg}");
}

/// A socket that exists but cannot be opened is a different problem with a
/// different fix, and the old message could not tell them apart.
#[cfg(unix)]
#[test]
fn a_denied_socket_says_so_rather_than_suggesting_enabling_it() {
	let e = super::socket_error(
		"/tmp/s.sock",
		std::io::Error::from(std::io::ErrorKind::PermissionDenied),
	);
	let msg = e.to_string();
	assert!(msg.contains("/tmp/s.sock"), "{msg}");
	assert!(msg.contains("cannot be opened"), "{msg}");
	assert!(
		!msg.contains("enable --now"),
		"enabling the socket does not fix a permission problem: {msg}"
	);
}

/// The io::ErrorKind must survive, since it is what distinguishes the two.
#[cfg(unix)]
#[test]
fn the_error_kind_is_preserved() {
	let e = super::socket_error("/x", std::io::Error::from(std::io::ErrorKind::NotFound));
	assert!(
		matches!(&e, super::super::PodmanError::Connect(io) if io.kind() == std::io::ErrorKind::NotFound)
	);
}
