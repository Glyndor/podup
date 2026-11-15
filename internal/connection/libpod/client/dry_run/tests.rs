//! The dry-run gate lets a read through and refuses every other method,
//! before anything reaches the socket.

use hyper::Method;

use super::super::{Client, PodmanError};

const PATH: &str = "/v5.0.0/libpod/containers/create";

fn refused(client: &Client, method: Method) -> bool {
	matches!(
		client.refuse_in_dry_run(&method, PATH),
		Err(PodmanError::DryRun { .. })
	)
}

#[test]
fn a_client_that_is_not_in_dry_run_refuses_nothing() {
	let client = Client::new("/nonexistent.sock");
	for method in [Method::GET, Method::POST, Method::PUT, Method::DELETE] {
		assert!(!refused(&client, method));
	}
	assert!(!client.is_dry_run());
}

#[test]
fn a_dry_run_client_lets_reads_through() {
	let client = Client::new("/nonexistent.sock").with_dry_run(true);
	assert!(client.is_dry_run());
	assert!(!refused(&client, Method::GET));
	assert!(!refused(&client, Method::HEAD));
}

#[test]
fn a_dry_run_client_refuses_every_other_method() {
	let client = Client::new("/nonexistent.sock").with_dry_run(true);
	for method in [
		Method::POST,
		Method::PUT,
		Method::DELETE,
		Method::PATCH,
		Method::OPTIONS,
		Method::CONNECT,
		Method::TRACE,
	] {
		assert!(refused(&client, method.clone()), "{method} was not refused");
	}
}

#[test]
fn the_refusal_names_the_method_and_the_path() {
	let client = Client::new("/nonexistent.sock").with_dry_run(true);
	let text = client
		.refuse_in_dry_run(&Method::DELETE, PATH)
		.unwrap_err()
		.to_string();
	assert!(text.contains("--dry-run"), "{text}");
	assert!(text.contains("DELETE") && text.contains(PATH), "{text}");
}

#[tokio::test]
async fn a_mutating_call_is_refused_without_opening_the_socket() {
	// The path does not exist: a request that was sent would fail with a
	// connection error, not with the dry-run refusal.
	let client = Client::new("/nonexistent/podup-dry-run.sock").with_dry_run(true);
	let err = client.post_empty_ok("/v5.0.0/libpod/pods/x/start").await;
	assert!(matches!(err, Err(PodmanError::DryRun { .. })), "{err:?}");
	let err = client.delete_ok("/v5.0.0/libpod/containers/x").await;
	assert!(matches!(err, Err(PodmanError::DryRun { .. })), "{err:?}");
}
