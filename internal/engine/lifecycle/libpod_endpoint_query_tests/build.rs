use super::*;

/// Spot-check the constants and the request path used by the build
/// endpoint. The build endpoint itself is covered by
/// `engine::build::query_tests::build_query_carries_layers_true`;
/// what is pinned here is that the URL the build request is built
/// against still points at the libpod `/build` path (a regression
/// here would mean the compensation went to the wrong place).
#[tokio::test]
async fn build_request_path_targets_libpod_build() {
	let fake = fake_podman::start_replying(move |method, target| {
		if method == "POST" && target.contains("/build?") {
			FakeReply::ChunkedEnd(vec![
				"{\"stream\":\"--> sha256:1111111111111111111111111111111111111111111111111111111111111111\\n\"}\n".to_string(),
				"{\"stream\":\"Successfully tagged proj/img:1\\n\"}\n".to_string(),
			])
		} else if method == "POST" && target.contains("/images/") && target.contains("/tag") {
			FakeReply::Body(200, String::new())
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.into())
		}
	});

	let dir = tempfile::tempdir().unwrap();
	let ctx = dir.path().to_path_buf();
	std::fs::write(ctx.join("Dockerfile"), b"FROM alpine:latest\nRUN echo hi\n").unwrap();
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), ctx.clone());
	let file = crate::parse_str(
		"services:\n  app:\n    image: proj/img:1\n    build:\n      context: .\n",
	)
	.unwrap();
	engine
		.build_all_with_options(&file, &[], &crate::engine::BuildOptions::default())
		.await
		.expect("a build the fake accepts succeeds");

	// The full build path lands at `${API_PREFIX}/build?...`; a regression
	// that moved the target outside `/libpod/` would let the docker
	// compat handler answer again and the `layers=true` compensation be
	// for nothing.
	let requests = fake.requests.lock().unwrap();
	assert!(
		requests
			.iter()
			.any(|r| { r.starts_with("POST ") && r.contains(&format!("{API_PREFIX}/build?")) }),
		"a POST to `{API_PREFIX}/build?...` must be issued: {requests:?}"
	);
}
