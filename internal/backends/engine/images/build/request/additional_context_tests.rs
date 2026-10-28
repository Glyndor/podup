//! Unit tests on the additional build contexts the build sends.
//!
//! The libpod build endpoint reads the contexts it fetches on its own —
//! an image or a URL — from the query, and the local ones from the request
//! body: the main context from the `MainContext` part and each additional
//! directory from a part named `build-context-NAME`. A local context named
//! only in the query was dropped, and the build could not use the context
//! it was given.

#[cfg(unix)]
mod tests {
	use std::io::Read;

	use crate::engine::fake_podman::{self, FakeReply};
	use crate::engine::Engine;

	/// The index of the `POST /libpod/build` request among the requests the
	/// fake answered.
	fn build_request(requests: &[String]) -> usize {
		requests
			.iter()
			.position(|r| r.starts_with("POST ") && r.contains("/build?"))
			.expect("a /build request was issued")
	}

	/// The offset of `needle` in `haystack`, from `from` on.
	fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
		if needle.is_empty() || from >= haystack.len() {
			return None;
		}
		haystack[from..]
			.windows(needle.len())
			.position(|w| w == needle)
			.map(|i| i + from)
	}

	/// The parts of a multipart body, as `(name, payload)`.
	fn parts(body: &[u8], boundary: &str) -> Vec<(String, Vec<u8>)> {
		let header_delimiter = format!("--{boundary}\r\n");
		let payload_delimiter = format!("\r\n--{boundary}");
		let mut out = Vec::new();
		let mut pos = 0usize;
		while let Some(start) = find(body, header_delimiter.as_bytes(), pos) {
			let after = start + header_delimiter.len();
			let Some(header_end) = find(body, b"\r\n\r\n", after) else {
				break;
			};
			let headers = String::from_utf8_lossy(&body[after..header_end]).into_owned();
			let name = headers
				.lines()
				.find_map(|l| l.strip_prefix("Content-Disposition: form-data; name=\""))
				.and_then(|l| l.split('"').next())
				.unwrap_or_default()
				.to_string();
			let payload_start = header_end + 4;
			let payload_end =
				find(body, payload_delimiter.as_bytes(), payload_start).unwrap_or(body.len());
			out.push((name, body[payload_start..payload_end].to_vec()));
			pos = payload_end + 2;
		}
		out
	}

	/// The names of the files a gzipped tar carries.
	fn tar_entries(payload: &[u8]) -> Vec<String> {
		use flate2::read::GzDecoder;
		let mut raw = Vec::new();
		GzDecoder::new(payload)
			.read_to_end(&mut raw)
			.expect("the part payload is a gzip stream");
		let mut archive = tar::Archive::new(raw.as_slice());
		archive
			.entries()
			.expect("the part payload is a tar archive")
			.filter_map(|e| e.ok())
			.filter_map(|e| e.path().ok().map(|p| p.to_string_lossy().into_owned()))
			.collect()
	}

	#[tokio::test]
	async fn a_local_additional_context_travels_in_its_own_part() {
		let dir = tempfile::tempdir().expect("tempdir");
		let base = dir.path().to_path_buf();
		std::fs::write(base.join("Dockerfile"), b"FROM alpine\n").expect("Dockerfile");
		std::fs::create_dir(base.join("resources")).expect("resources directory");
		std::fs::write(base.join("resources").join("data.txt"), b"payload\n")
			.expect("resource file");

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
			"services:\n  app:\n    image: proj/img:1\n    build:\n      context: .\n      \
			 additional_contexts:\n        resources: ./resources\n",
		)
		.unwrap();
		engine
			.build_all_with_options(&file, &[], &crate::engine::BuildOptions::default())
			.await
			.expect("a build the fake accepts succeeds");

		let requests = fake.requests.lock().unwrap().clone();
		let index = build_request(&requests);
		let content_types = fake.content_types.lock().unwrap().clone();
		let content_type = content_types.get(index).cloned().unwrap_or_default();
		let boundary = content_type
			.strip_prefix("multipart/form-data; boundary=")
			.unwrap_or_else(|| {
				panic!("the build request must be multipart to carry the local contexts: {content_type}")
			})
			.to_string();

		let bodies = fake.bodies.lock().unwrap().clone();
		let parts = parts(&bodies[index], &boundary);

		let resources = parts
			.iter()
			.find(|(name, _)| name == "build-context-resources")
			.unwrap_or_else(|| {
				panic!("the local additional context must travel in its own part: {parts:?}")
			});
		let entries = tar_entries(&resources.1);
		assert!(
			entries.iter().any(|e| e == "data.txt"),
			"the part must carry the files of the local context: {entries:?}"
		);

		// The main context travels in its own part too.
		assert!(
			parts.iter().any(|(name, _)| name == "MainContext"),
			"the main context must travel in the MainContext part: {parts:?}"
		);
	}
}
