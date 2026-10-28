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
	/// references, so a registry cache travels as its reference.
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
			serde_json::json!(["example/app:cache", "other.example/cache:latest"]),
			"cachefrom= must carry the image reference of every registry cache"
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
}
