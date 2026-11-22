//! Unit tests for [`meets_minimum`] and for the status-decoding pair
//! ([`Client::check_status`](super::status::Client::check_status) and
//! [`Client::parse_error_message`](super::status::Client::parse_error_message)).

use hyper::StatusCode;

use super::status::{meets_floor, meets_minimum};
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

/// Releases below the floor are refused, whichever of the three parts is
/// short: an older major, the floor's major with an older minor, and the whole
/// of Podman 5, which podup no longer supports.
#[test]
fn meets_minimum_rejects_everything_below_the_floor() {
	for below in [
		"5.0.0", "5.4.2", "5.7.0", "5.8.5", "5.99.99", "6.0.0", "6.1.4", "6.1.99", "4.9.3",
		"3.4.4", "0.0.0",
	] {
		assert!(
			!meets_minimum(below),
			"{below} is below 6.2.0 and was accepted"
		);
	}
}

/// The floor itself and every release above it are accepted, with no upper
/// bound. `6.10.0` and `6.2.10` sit above `6.2.0` and only a numeric
/// comparison says so; compared as text both would read as lower.
#[test]
fn meets_minimum_accepts_the_floor_and_everything_above() {
	for ok in [
		"6.2.0", "6.2.1", "6.2.10", "6.3.0", "6.10.0", "6.100.0", "7.0.0", "10.0.0", "99.99.99",
	] {
		assert!(
			meets_minimum(ok),
			"{ok} is at or above 6.2.0 and was refused"
		);
	}
}

/// The patch part decides when the floor has one. The real floor ends in `.0`,
/// so no real version can fall below it on the patch alone; a floor with a
/// patch shows the third part is read and compared.
#[test]
fn meets_floor_compares_the_patch_last() {
	let floor = (6, 2, 4);
	assert!(!meets_floor("6.2.3", floor));
	assert!(meets_floor("6.2.4", floor));
	assert!(meets_floor("6.2.10", floor));
	assert!(
		meets_floor("6.3.0", floor),
		"a higher minor outranks the patch"
	);
	assert!(!meets_floor("6.1.99", floor));
	assert!(
		meets_floor("7.0.0", floor),
		"a higher major outranks the rest"
	);
}

/// A leading `v` and surrounding whitespace are tolerated: some libpod builds
/// report `v6.2.0`.
#[test]
fn meets_minimum_tolerates_a_v_prefix_and_whitespace() {
	assert!(meets_minimum("v6.2.0"));
	assert!(meets_minimum(" 6.2.0 "));
	assert!(meets_minimum("\t6.10.0\n"));
	assert!(!meets_minimum("v6.1.4"));
	assert!(!meets_minimum("v5.7.0"));
	assert!(!meets_minimum(" 5.7.0 "));
}

/// A pre-release or build suffix after the patch does not hide the release it
/// leads up to: a release candidate of 6.2.0 reports `6.2.0-rc1`, a build of
/// the next release reports `6.3.0-dev`.
#[test]
fn meets_minimum_reads_the_release_under_a_suffix() {
	assert!(meets_minimum("6.2.0-rc1"));
	assert!(meets_minimum("6.3.0-dev"));
	assert!(meets_minimum("6.2.0+build.7"));
	assert!(!meets_minimum("6.1.4-dev"));
	assert!(!meets_minimum("5.8.5-rc1"));
	assert!(
		!meets_minimum("6.2-dev"),
		"a suffix cannot stand in for the patch"
	);
}

/// A missing or malformed `Libpod-API-Version` fails closed: we never assume a
/// compatible server from an unparseable value.
#[test]
fn meets_minimum_handles_malformed_and_empty() {
	for unreadable in [
		"",
		"   ",
		"not-a-version",
		".5",
		"6",
		"6.2",
		"6.",
		"6.2.",
		"6..0",
		".2.0",
		"6.2.0.1",
		"6.2.x",
		"six.2.0",
		"6.2.0 6.3.0",
		"99999999999999999999.0.0",
	] {
		assert!(
			!meets_minimum(unreadable),
			"accepted unreadable version {unreadable:?}"
		);
	}
}

/// Only one `v` prefix and plain runs of digits count as version parts, so a
/// repeated prefix or a sign never reads as a compatible server.
#[test]
fn meets_minimum_refuses_repeated_prefixes_and_signed_parts() {
	for hostile in [
		"vv6.2.0", "vvv7.1.0", "+6.2.0", "-6.2.0", "v+6.2.0", "6x.2.0", "6.+2.0", "6.2.+0",
		"6.-2.0", "6.2.-1", "7.0.0.0", "v",
	] {
		assert!(
			!meets_minimum(hostile),
			"accepted malformed version {hostile:?}"
		);
	}
}
