//! A build asks libpod to refresh its base images with the pull policy the
//! endpoint reads. The endpoint has no boolean `pull` parameter, so sending
//! one leaves a stale local base in place.

use crate::engine::fake_podman::{self, FakeReply};
use crate::engine::{BuildOptions, Engine};

/// Build `compose` against a fake libpod and return the build query string.
async fn build_query(compose: &str, opts: BuildOptions, always: bool) -> String {
	let dir = tempfile::tempdir().expect("tempdir");
	std::fs::write(dir.path().join("Dockerfile"), b"FROM alpine:latest\n").expect("Dockerfile");
	let fake = fake_podman::start_replying(|method, target| {
		if method == "POST" && target.contains("/build?") {
			FakeReply::ChunkedEnd(vec![
				"{\"stream\":\"--> sha256:1111111111111111111111111111111111111111111111111111111111111111\\n\"}\n".to_string(),
			])
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let mut engine = Engine::with_base_dir(fake.client(), "proj".into(), dir.path().to_path_buf());
	if always {
		engine.pull_policy_override = Some("always".into());
	}
	let file = crate::parse_str(compose).expect("the compose file parses");
	engine
		.build_all_with_options(&file, &[], &opts)
		.await
		.expect("a build the fake accepts succeeds");
	let requests = fake.requests.lock().unwrap().clone();
	let target = requests
		.iter()
		.find(|r| r.starts_with("POST ") && r.contains("/build?"))
		.expect("a /build request was sent")
		.split_whitespace()
		.nth(1)
		.expect("a request line has a target")
		.to_string();
	target
		.split_once('?')
		.expect("a query string")
		.1
		.to_string()
}

fn params(query: &str) -> Vec<&str> {
	query.split('&').collect()
}

const PLAIN: &str = "services:\n  a:\n    build: .\n";

#[tokio::test]
async fn a_build_without_a_pull_request_sends_no_pull_parameter() {
	let query = build_query(PLAIN, BuildOptions::default(), false).await;
	assert!(!query.contains("pull"), "{query}");
}

#[tokio::test]
async fn the_pull_flag_sends_the_always_pull_policy() {
	let opts = BuildOptions::new(false, true, vec![], false);
	let query = build_query(PLAIN, opts, false).await;
	assert!(params(&query).contains(&"pullpolicy=always"), "{query}");
	assert!(!params(&query).contains(&"pull=true"), "{query}");
}

#[tokio::test]
async fn build_pull_sends_the_always_pull_policy() {
	let compose = "services:\n  a:\n    build:\n      context: .\n      pull: true\n";
	let query = build_query(compose, BuildOptions::default(), false).await;
	assert!(params(&query).contains(&"pullpolicy=always"), "{query}");
}

#[tokio::test]
async fn pull_policy_always_on_the_service_refreshes_the_base_image() {
	let compose = "services:\n  a:\n    build: .\n    pull_policy: always\n";
	let query = build_query(compose, BuildOptions::default(), false).await;
	assert!(params(&query).contains(&"pullpolicy=always"), "{query}");
}

#[tokio::test]
async fn an_always_policy_forced_for_the_run_refreshes_the_base_image() {
	let query = build_query(PLAIN, BuildOptions::default(), true).await;
	assert!(params(&query).contains(&"pullpolicy=always"), "{query}");
}

#[tokio::test]
async fn other_pull_policies_leave_the_base_image_alone() {
	for policy in ["missing", "never", "build"] {
		let compose = format!("services:\n  a:\n    build: .\n    pull_policy: {policy}\n");
		let query = build_query(&compose, BuildOptions::default(), false).await;
		assert!(!query.contains("pull"), "{policy}: {query}");
	}
}
