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
fn build_request_sets_content_type_when_given() {
	use bytes::Bytes;
	use http_body_util::Full;
	use hyper::Method;
	let req = Client::build_request(
		Method::POST,
		"/libpod/secrets/create",
		Full::new(Bytes::new()),
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

#[test]
fn build_request_rejects_unparseable_path() {
	use bytes::Bytes;
	use http_body_util::Full;
	use hyper::Method;
	// A control character makes `http://localhost<path>` an invalid URI, which
	// must surface as a structured Api error rather than panicking.
	let err = Client::build_request(
		Method::GET,
		"/libpod/bad\u{7f}path",
		Full::new(Bytes::new()),
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
/// in the message — the guard that stops a stalled or silent socket (whether
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
