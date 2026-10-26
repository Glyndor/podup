//! Unit tests for the transport-level helpers every request shares: the
//! timeout wrapper that bounds a wait on the Podman socket, and the size hint
//! a boxed `Full` body must keep so hyper does not fall back to chunked
//! transfer encoding (#1097).

use super::transport::full;
use super::Client;

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
	let body = full(payload.clone());
	assert_eq!(
		body.size_hint().exact(),
		Some(payload.len() as u64),
		"a boxed Full body must keep its exact size, or hyper sends it chunked"
	);
}
