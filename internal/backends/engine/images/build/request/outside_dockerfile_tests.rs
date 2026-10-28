//! Unit tests on a Dockerfile that lives outside the build context.
//!
//! Compose allows `build.dockerfile` to leave the context: its illustrative
//! example uses `dockerfile: ../backend.Dockerfile` with `context: backend`,
//! and the file is then a sibling of the context directory. The daemon
//! resolves the `dockerfile` parameter against the extracted context, where
//! such a path is not there, so podup reads the file and ships it inside the
//! tar under a name that resolves inside the context.

#[cfg(unix)]
mod tests {
	use std::io::Read;

	use crate::engine::fake_podman::{self, FakeReply};
	use crate::engine::Engine;

	/// The `dockerfile=` parameter of the build request, percent-decoded.
	fn dockerfile_param(query: &str) -> Option<String> {
		for pair in query.split('&') {
			if let Some(value) = pair.strip_prefix("dockerfile=") {
				return Some(percent_decode(value));
			}
		}
		None
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

	/// The tar entries of the build request body, as `(name, bytes)`.
	fn body_entries(body: &[u8]) -> Vec<(String, Vec<u8>)> {
		use flate2::read::GzDecoder;
		let mut raw = Vec::new();
		GzDecoder::new(body)
			.read_to_end(&mut raw)
			.expect("the build context body is a gzip stream");
		let mut archive = tar::Archive::new(raw.as_slice());
		let mut out = Vec::new();
		for entry in archive.entries().expect("the body is a tar archive") {
			let mut entry = entry.expect("a readable tar entry");
			let name = entry
				.path()
				.expect("a tar entry path")
				.to_string_lossy()
				.into_owned();
			let mut content = Vec::new();
			entry
				.read_to_end(&mut content)
				.expect("a readable tar entry body");
			out.push((name, content));
		}
		out
	}

	#[tokio::test]
	async fn a_dockerfile_outside_the_context_is_shipped_inside_the_tar() {
		let dir = tempfile::tempdir().expect("tempdir");
		let base = dir.path().to_path_buf();
		std::fs::create_dir(base.join("backend")).expect("context directory");
		std::fs::write(base.join("backend").join("app.txt"), b"x").expect("context file");
		let outside = b"FROM alpine\nRUN echo outside\n";
		std::fs::write(base.join("backend.Dockerfile"), outside).expect("outside Dockerfile");

		let fake = fake_podman::start_replying(|method, target| {
			if method == "POST" && target.contains("/build?") {
				FakeReply::ChunkedEnd(vec![
					"{\"stream\":\"--> sha256:1111111111111111111111111111111111111111111111111111111111111111\\n\"}\n".to_string(),
				])
			} else {
				FakeReply::Body(404, r#"{"message":"not found"}"#.to_string())
			}
		});
		let engine = Engine::with_base_dir(fake.client(), "proj".into(), base.clone());

		let file = crate::parse_str(
			"services:\n  app:\n    image: proj/img:1\n    build:\n      context: backend\n      \
			 dockerfile: ../backend.Dockerfile\n",
		)
		.unwrap();
		engine
			.build_all_with_options(&file, &[], &crate::engine::BuildOptions::default())
			.await
			.expect("a build the fake accepts succeeds");

		let requests = fake.requests.lock().unwrap().clone();
		let bodies = fake.bodies.lock().unwrap().clone();
		let index = requests
			.iter()
			.position(|r| r.starts_with("POST ") && r.contains("/build?"))
			.expect("a /build request was issued");
		let target = requests[index].split_whitespace().nth(1).unwrap();
		let query = target
			.split_once('?')
			.expect("the build target carries a query");

		// The Dockerfile the daemon is told to use must name a path that
		// resolves inside the extracted context.
		let dockerfile = dockerfile_param(query.1).expect("the build query carries a dockerfile=");
		assert!(
			!dockerfile.contains('/') && !dockerfile.contains('\\'),
			"the dockerfile parameter must name a file inside the context, got {dockerfile:?}"
		);
		assert!(
			!dockerfile.split(['/', '\\']).any(|c| c == ".."),
			"the dockerfile parameter must not leave the context, got {dockerfile:?}"
		);

		// And the file the build was asked to use must be in the tar, under
		// that name.
		let entries = body_entries(&bodies[index]);
		let shipped: Vec<&(String, Vec<u8>)> = entries
			.iter()
			.filter(|(name, _)| *name == dockerfile)
			.collect();
		assert_eq!(
			shipped.len(),
			1,
			"the tar must carry exactly one entry named {dockerfile:?}, found {shipped:?}"
		);
		assert_eq!(
			shipped[0].1, outside,
			"the shipped Dockerfile must be the file outside the context"
		);
	}
}
