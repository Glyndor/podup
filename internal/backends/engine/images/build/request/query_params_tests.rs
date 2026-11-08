//! Unit tests on the build query's own parameters.
//!
//! The libpod build endpoint reads several of them as JSON: `extrahosts` is
//! an array of `host:ip` entries, and `cachefrom`/`cacheto` are arrays of
//! image references. A value that is not JSON in that shape is rejected by
//! the daemon before the build starts.

#[cfg(unix)]
mod tests {
	use crate::engine::fake_podman::{self, FakeReply};
	use crate::engine::Engine;

	/// A fake libpod that answers `/build` with a one-line stream and records
	/// the request line of every call.
	fn start_capture(project: &str) -> (tempfile::TempDir, fake_podman::FakePodman, Engine) {
		let dir = tempfile::tempdir().expect("tempdir");
		std::fs::write(
			dir.path().join("Dockerfile"),
			b"FROM alpine:latest\nRUN echo hi\n",
		)
		.expect("Dockerfile");
		let fake = fake_podman::start_replying(|method, target| {
			if method == "POST" && target.contains("/build?") {
				FakeReply::ChunkedEnd(vec![
					"{\"stream\":\"--> sha256:1111111111111111111111111111111111111111111111111111111111111111\\n\"}\n".to_string(),
				])
			} else {
				FakeReply::Body(404, r#"{"message":"not found"}"#.to_string())
			}
		});
		let engine = Engine::with_base_dir(fake.client(), project.into(), dir.path().to_path_buf());
		(dir, fake, engine)
	}

	/// The query string of the single `POST /libpod/build` request the build
	/// issued.
	async fn build_query(engine: &Engine, fake: &fake_podman::FakePodman, compose: &str) -> String {
		let file = crate::parse_str(compose).expect("the compose file parses");
		engine
			.build_all_with_options(&file, &[], &crate::engine::BuildOptions::default())
			.await
			.expect("a build the fake accepts succeeds");
		let requests = fake.requests.lock().unwrap().clone();
		let target = requests
			.iter()
			.find(|r| r.starts_with("POST ") && r.contains("/build?"))
			.expect("a /build request was issued")
			.split_whitespace()
			.nth(1)
			.expect("a request line has at least two tokens");
		target
			.split_once('?')
			.expect("the build target carries a query string")
			.1
			.to_string()
	}

	fn percent_decode(input: &str) -> String {
		let bytes = input.as_bytes();
		let mut out = Vec::with_capacity(bytes.len());
		let mut i = 0;
		while i < bytes.len() {
			match bytes[i] {
				b'%' if i + 2 < bytes.len() => {
					let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
						.expect("percent escape is ASCII hex");
					out.push(u8::from_str_radix(hex, 16).expect("percent escape hex"));
					i += 3;
				}
				b'+' => {
					out.push(b' ');
					i += 1;
				}
				other => {
					out.push(other);
					i += 1;
				}
			}
		}
		String::from_utf8(out).expect("query is UTF-8")
	}

	/// The percent-decoded value of the first `key=` parameter of the query.
	fn param(query: &str, key: &str) -> Option<String> {
		for pair in query.split('&') {
			if let Some(value) = pair.strip_prefix(key) {
				return Some(percent_decode(value));
			}
		}
		None
	}

	/// The daemon reads `extrahosts` as a JSON array of `host:ip` entries.
	/// A comma-separated list is not that shape, and the build is rejected
	/// before it starts.
	#[tokio::test]
	async fn extra_hosts_reach_the_build_query_as_a_json_array() {
		let (_dir, fake, engine) = start_capture("proj");

		let query = build_query(
			&engine,
			&fake,
			"services:\n  app:\n    image: proj/img:1\n    build:\n      context: .\n      \
			 extra_hosts:\n        - \"example.test:127.0.0.1\"\n        - \
			 \"other.test:10.0.0.2\"\n",
		)
		.await;

		let raw = param(&query, "extrahosts=").expect("extrahosts= is sent");
		let value: serde_json::Value =
			serde_json::from_str(&raw).expect("extrahosts= must be a JSON array");
		assert_eq!(
			value,
			serde_json::json!(["example.test:127.0.0.1", "other.test:10.0.0.2"]),
			"extrahosts= must carry every entry of build.extra_hosts"
		);
	}

	/// Compose writes the caches with the global
	/// `[NAME|type=TYPE[,KEY=VALUE]]` syntax, where a bare name is a shortcut
	/// for `type=registry,ref=NAME`. The endpoint reads a JSON array of image
	/// references without a tag or digest, so a registry cache travels as the
	/// repository its reference names.
	#[tokio::test]
	async fn a_registry_cache_reaches_the_build_query_as_an_image_reference() {
		let (_dir, fake, engine) = start_capture("proj");

		let query = build_query(
			&engine,
			&fake,
			"services:\n  app:\n    image: proj/img:1\n    build:\n      context: .\n      \
			 cache_from:\n        - type=registry,ref=example/app:cache\n        - \
			 other.example/cache:latest\n",
		)
		.await;

		let raw = param(&query, "cachefrom=").expect("cachefrom= is sent");
		let value: serde_json::Value =
			serde_json::from_str(&raw).expect("cachefrom= must be a JSON array");
		assert_eq!(
			value,
			serde_json::json!(["example/app", "other.example/cache"]),
			"cachefrom= must carry the repository of every registry cache, without its tag"
		);
	}

	/// A cache type with no libpod equivalent is ignored: the Compose
	/// contract says unsupported caches do not prevent the build.
	#[tokio::test]
	async fn a_cache_type_without_an_equivalent_is_ignored() {
		let (_dir, fake, engine) = start_capture("proj");

		let query = build_query(
			&engine,
			&fake,
			"services:\n  app:\n    image: proj/img:1\n    build:\n      context: .\n      \
			 cache_from:\n        - type=local,src=path/to/cache\n      cache_to:\n        - \
			 type=local,dest=path/to/cache\n",
		)
		.await;

		assert_eq!(
			param(&query, "cachefrom="),
			None,
			"a cache source with no libpod equivalent must not reach the query: {query}"
		);
		assert_eq!(
			param(&query, "cacheto="),
			None,
			"a cache target with no libpod equivalent must not reach the query: {query}"
		);
	}

	/// The daemon finds the Dockerfile of a Git context by its file name
	/// alone, so a Dockerfile in a folder travels as the folder of the context
	/// and a bare file name.
	#[tokio::test]
	async fn a_dockerfile_folder_of_a_git_context_moves_into_the_remote() {
		let (_dir, fake, engine) = start_capture("proj");

		let query = build_query(
			&engine,
			&fake,
			"services:\n  app:\n    image: proj/img:1\n    build:\n      context: \
			 \"https://example.test/org/repo.git#main\"\n      dockerfile: docker/Dockerfile\n",
		)
		.await;

		assert_eq!(
			param(&query, "remote=").as_deref(),
			Some("https://example.test/org/repo.git#main:docker")
		);
		assert_eq!(param(&query, "dockerfile=").as_deref(), Some("Dockerfile"));
	}

	/// A Git context with a Dockerfile at its top is sent as written.
	#[tokio::test]
	async fn a_git_context_without_a_dockerfile_folder_is_sent_as_written() {
		let (_dir, fake, engine) = start_capture("proj");

		let query = build_query(
			&engine,
			&fake,
			"services:\n  app:\n    image: proj/img:1\n    build:\n      context: \
			 \"https://example.test/org/repo.git#main:app\"\n      dockerfile: Dockerfile.prod\n",
		)
		.await;

		assert_eq!(
			param(&query, "remote=").as_deref(),
			Some("https://example.test/org/repo.git#main:app")
		);
		assert_eq!(
			param(&query, "dockerfile=").as_deref(),
			Some("Dockerfile.prod")
		);
	}

	/// With no `build.platforms`, the build is for the platform the service
	/// declares.
	#[tokio::test]
	async fn a_service_platform_is_the_build_platform_when_none_is_listed() {
		let (_dir, fake, engine) = start_capture("proj");

		let query = build_query(
			&engine,
			&fake,
			"services:\n  app:\n    image: proj/img:1\n    platform: linux/arm64\n    build:\n      \
			 context: .\n",
		)
		.await;

		assert_eq!(param(&query, "platform=").as_deref(), Some("linux/arm64"));
	}

	/// `build.platforms` is the more specific setting and wins.
	#[tokio::test]
	async fn build_platforms_win_over_the_service_platform() {
		let (_dir, fake, engine) = start_capture("proj");

		let query = build_query(
			&engine,
			&fake,
			"services:\n  app:\n    image: proj/img:1\n    platform: linux/arm64\n    build:\n      \
			 context: .\n      platforms:\n        - linux/amd64\n",
		)
		.await;

		assert_eq!(param(&query, "platform=").as_deref(), Some("linux/amd64"));
	}

	/// A `service:` context reaches the daemon as the image that service
	/// builds, not as a path on disk.
	#[tokio::test]
	async fn a_service_context_reaches_the_build_query_as_an_image() {
		let (dir, fake, engine) = start_capture("proj");
		std::fs::create_dir(dir.path().join("base")).expect("base dir");
		std::fs::write(
			dir.path().join("base").join("Dockerfile"),
			b"FROM scratch\n",
		)
		.expect("base Dockerfile");

		let file = crate::parse_str(
			"services:\n  base:\n    image: proj/base:1\n    build: ./base\n  app:\n    image: \
			 proj/app:1\n    build:\n      context: .\n      additional_contexts:\n        \
			 basectx: service:base\n",
		)
		.expect("the compose file parses");
		engine
			.build_all_with_options(
				&file,
				&["app".to_string()],
				&crate::engine::BuildOptions::default(),
			)
			.await
			.expect("a build the fake accepts succeeds");

		let requests = fake.requests.lock().unwrap().clone();
		let builds: Vec<&String> = requests
			.iter()
			.filter(|r| r.starts_with("POST ") && r.contains("/build?"))
			.collect();
		assert_eq!(
			builds.len(),
			2,
			"the base is built before the app: {builds:?}"
		);
		let base_query = builds[0].split_once('?').expect("a query").1;
		assert!(param(base_query, "t=").is_some_and(|t| t.ends_with("proj/base:1")));
		let app_query = builds[1].split_once('?').expect("a query").1;
		assert_eq!(
			param(app_query, "additionalbuildcontexts=").as_deref(),
			Some("basectx=image:proj/base:1")
		);
	}
}
