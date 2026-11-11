use crate::engine::fake_podman::{self, FakeReply};
use crate::engine::Engine;

/// A fake host that already has every image and accepts a build.
fn builds_on_a_warm_host(chunks: Vec<String>) -> fake_podman::FakePodman {
	fake_podman::start_replying(move |method, target| {
		if method == "POST" && target.contains("/build?") {
			FakeReply::ChunkedEnd(chunks.clone())
		} else if method == "POST" && target.contains("/tag") {
			FakeReply::Body(200, String::new())
		} else if method == "GET" && target.contains("/images/") && target.contains("/json") {
			FakeReply::Body(200, r#"{"Id":"sha256:cafe"}"#.to_string())
		} else if method == "GET" && target.contains("/containers/json") {
			FakeReply::Body(200, "[]".to_string())
		} else if method == "POST" && target.contains("/containers/create") {
			FakeReply::Body(200, "{}".to_string())
		} else if method == "POST" && target.contains("/start") {
			FakeReply::Body(200, String::new())
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.to_string())
		}
	})
}

async fn builds_issued(pull_policy: &str, no_build: bool) -> usize {
	let ctx = tempfile::tempdir().expect("tempdir");
	std::fs::write(ctx.path().join("Dockerfile"), b"FROM scratch\n").expect("write Dockerfile");
	let fake = builds_on_a_warm_host(vec![
		"{\"stream\":\"Successfully tagged localhost/pp-build:1\\n\"}\n".to_string(),
	]);
	let mut engine = Engine::with_base_dir(fake.client(), "proj".into(), ctx.path().to_path_buf());
	engine.no_warn = true;
	engine.no_build = no_build;
	let compose = crate::parse_str(&format!(
		"services:\n  app:\n    image: localhost/pp-build:1\n    build:\n      context: .\n    pull_policy: {pull_policy}\n"
	))
	.unwrap();
	engine
		.up_with_options(&compose, false, &[], &[], false, false, false, false)
		.await
		.expect("up succeeds");
	let seen = fake.requests.lock().unwrap();
	seen.iter().filter(|r| r.contains("/build?")).count()
}

/// `pull_policy: build` makes `up` rebuild a service whose image is already
/// present, without `--build`.
#[tokio::test]
async fn pull_policy_build_rebuilds_a_present_image_on_up() {
	assert_eq!(builds_issued("build", false).await, 1);
}

/// The controls: the default policy leaves a present image alone, and
/// `--no-build` still suppresses the build `pull_policy: build` asks for.
#[tokio::test]
async fn present_image_is_not_rebuilt_without_pull_policy_build() {
	assert_eq!(builds_issued("missing", false).await, 0);
	assert_eq!(builds_issued("build", true).await, 0);
}
