//! Unit tests for [`meets_minimum`] and for the status-decoding pair
//! ([`Client::check_status`](super::status::Client::check_status) and
//! [`Client::parse_error_message`](super::status::Client::parse_error_message)).

use hyper::StatusCode;

use super::status::meets_minimum;
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
	let err =
		Client::check_status(StatusCode::INTERNAL_SERVER_ERROR, b"plain text error").unwrap_err();
	assert!(err.is_status(500));
	assert!(err.to_string().contains("plain text error"));
	assert!(
		matches!(&err, crate::connection::libpod::client::PodmanError::Api { message, .. } if message == "plain text error"),
		"a non-JSON body must be kept verbatim: {err:?}"
	);
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
// version gate tests
// ---------------------------------------------------------------------------

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
