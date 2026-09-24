//! Unit tests on the query string the build request sends to libpod.
//!
//! The build query carries podup's ownership labels on every image the
//! build produced, including the intermediate stage images `labels=`
//! leaves unlabelled. These tests pin that contract on the wire shape:
//! `labels=` carries podup's keys, `layerLabel=` repeats each one as its
//! own url-encoded `key=value` parameter, and a user `build.labels`
//! named `podup.project` cannot override podup's value.
//!
//! Each test stands up a fake Podman, runs a single build through it,
//! and inspects the captured request line.

#[cfg(unix)]
mod tests {
	use std::sync::{Arc, Mutex};

	use crate::engine::fake_podman::{self, FakeReply};
	use crate::engine::Engine;

	/// A fake libpod that records every request line and answers `/build`
	/// with a one-line stream that satisfies `build_service`'s success
	/// path, plus `/images/{tag}/tag` for any extra tag the compose file
	/// declares. The returned `TempDir` keeps the build-context directory
	/// alive until the test drops it: cloning the path and discarding the
	/// directory drops the directory.
	///
	/// Each call returns a fresh `Arc<Mutex<Vec<String>>>` for request
	/// capture. The closure pushed into the fake captures the same Arc,
	/// so a test can read only its own request log without races against
	/// any other test in the binary.
	fn start_capture(
		project: &str,
	) -> (
		tempfile::TempDir,
		fake_podman::FakePodman,
		Engine,
		std::path::PathBuf,
		Arc<Mutex<Vec<String>>>,
	) {
		let dir = tempfile::tempdir().expect("tempdir");
		let ctx_path = dir.path().to_path_buf();
		std::fs::write(
			ctx_path.join("Dockerfile"),
			b"FROM alpine:latest\nRUN echo hi\n",
		)
		.expect("Dockerfile");
		let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
		let closure_requests = requests.clone();
		let fake = fake_podman::start_replying(move |method, target| {
			closure_requests
				.lock()
				.unwrap()
				.push(format!("{method} {target}"));
			if method == "POST" && target.contains("/build?") {
				FakeReply::ChunkedEnd(vec![
					"{\"stream\":\"--> sha256:1111111111111111111111111111111111111111111111111111111111111111\\n\"}\n".to_string(),
					"{\"stream\":\"Successfully tagged proj/img:1\\n\"}\n".to_string(),
				])
			} else if method == "POST" && target.contains("/images/") && target.contains("/tag") {
				FakeReply::Body(200, String::new())
			} else {
				FakeReply::Body(404, r#"{"message":"not found"}"#.to_string())
			}
		});
		let engine = Engine::with_base_dir(fake.client(), project.into(), ctx_path.clone());
		(dir, fake, engine, ctx_path, requests)
	}

	/// The single `POST /libpod/build` line the captured requests contain.
	/// Other calls may precede it (`/info` or similar); only the build call
	/// is under test here.
	fn build_target(requests: &[String]) -> &str {
		requests
			.iter()
			.find(|r| r.starts_with("POST ") && r.contains("/build?"))
			.expect("a /build request was issued")
			.split_whitespace()
			.nth(1)
			.expect("a request line has at least two tokens")
	}

	/// The `labels=...` JSON value, once percent-decoded. Empty when the
	/// build did not send a `labels=` parameter at all.
	fn labels_json(query: &str) -> String {
		let raw = extract(query, "labels=").expect("labels= is sent");
		percent_decode(raw)
	}

	/// All `layerLabel=...` parameter values in arrival order, each
	/// percent-decoded. Empty when the build sent none.
	fn layer_labels(query: &str) -> Vec<String> {
		let mut out = Vec::new();
		for pair in query.split('&') {
			if let Some(value) = pair.strip_prefix("layerLabel=") {
				out.push(percent_decode(value));
			}
		}
		out
	}

	fn extract<'a>(query: &'a str, key: &str) -> Option<&'a str> {
		for pair in query.split('&') {
			if let Some(value) = pair.strip_prefix(key) {
				return Some(value);
			}
		}
		None
	}

	/// How many whole query parameters equal `key`. A plain substring
	/// search is wrong: `query.1.matches("rm=true")` would also fire on
	/// `forcerm=true`, which is the very distinction this test pins.
	fn exact_param_count(query: &str, key: &str) -> usize {
		query.split('&').filter(|pair| *pair == key).count()
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

	#[tokio::test]
	async fn build_query_carries_podup_labels_and_layer_labels() {
		let (_dir, _fake, engine, _ctx, requests) = start_capture("proj");

		let file = crate::parse_str(
			"services:\n  app:\n    image: proj/img:1\n    build:\n      context: .\n",
		)
		.unwrap();
		engine
			.build_all_with_options(&file, &[], &crate::engine::BuildOptions::default())
			.await
			.expect("a build the fake accepts succeeds");

		let requests = requests.lock().unwrap().clone();
		let target = build_target(&requests);
		let query = target
			.split_once('?')
			.expect("the build target carries a query string");

		// `labels=` must carry both podup keys with this project's value.
		let labels_value: serde_json::Value =
			serde_json::from_str(&labels_json(query.1)).expect("labels= is JSON");
		let labels_map = labels_value.as_object().expect("labels= JSON is an object");
		assert_eq!(
			labels_map.get("podup.project").and_then(|v| v.as_str()),
			Some("proj"),
			"the labels= JSON does not carry podup.project=<project>: {labels_map:?}"
		);
		assert_eq!(
			labels_map.get("podup.service").and_then(|v| v.as_str()),
			Some("app"),
			"the labels= JSON does not carry podup.service=<service>: {labels_map:?}"
		);

		// Exactly two layerLabel= parameters, url-encoded, one per podup
		// key. They must come in deterministic order so a parser can split
		// them on `=` to recover each `key=value` cleanly.
		let layers = layer_labels(query.1);
		assert_eq!(
			layers,
			vec![
				"podup.project=proj".to_string(),
				"podup.service=app".to_string(),
			],
			"the two layerLabel= parameters are not what was expected: {layers:?}"
		);
	}

	#[tokio::test]
	async fn a_user_build_label_named_podup_project_does_not_override_podup() {
		let (_dir, _fake, engine, _ctx, requests) = start_capture("proj");

		// The compose file supplies a `build.labels: {podup.project: other}`
		// entry as a forgery attempt, plus an unrelated label the user
		// actually wants to keep.
		let file = crate::parse_str(
			"services:\n  app:\n    image: proj/img:1\n    build:\n      context: .\n      labels:\n        podup.project: other\n        user.keep: yes\n",
		)
		.unwrap();
		engine
			.build_all_with_options(&file, &[], &crate::engine::BuildOptions::default())
			.await
			.expect("a build the fake accepts succeeds");

		let requests = requests.lock().unwrap().clone();
		let target = build_target(&requests);
		let query = target.split_once('?').unwrap();

		let labels_value: serde_json::Value =
			serde_json::from_str(&labels_json(query.1)).expect("labels= is JSON");
		let labels_map = labels_value.as_object().expect("labels= JSON is an object");

		// podup's keys win.
		assert_eq!(
			labels_map.get("podup.project").and_then(|v| v.as_str()),
			Some("proj"),
			"a user `podup.project` label must not displace podup's value: {labels_map:?}"
		);
		assert_eq!(
			labels_map.get("podup.service").and_then(|v| v.as_str()),
			Some("app"),
			"the user's `podup.project` forgery must not displace podup.service: {labels_map:?}"
		);

		// The unrelated user label is still carried through.
		assert_eq!(
			labels_map.get("user.keep").and_then(|v| v.as_str()),
			Some("yes"),
			"unrelated user labels must still arrive: {labels_map:?}"
		);

		// The layerLabel side is unaffected: it carries podup's two keys
		// regardless of the user's labels.
		let layers = layer_labels(query.1);
		assert_eq!(
			layers,
			vec![
				"podup.project=proj".to_string(),
				"podup.service=app".to_string(),
			],
			"the layerLabel= parameters are unaffected by the user's labels: {layers:?}"
		);
	}

	/// `forcerm=true` and `rm=true` must both be present on the build
	/// query, and each must appear exactly once. `rm=true` removes
	/// intermediate containers only after a successful build;
	/// `forcerm=true` removes them after a failure too. Without
	/// `forcerm`, Podman 5.7.0 keeps a buildah working container for
	/// every failed build (measured 2026-09-24: 2 of 2 leaked without,
	/// 0 of 2 with). The exact-once count pins the wire shape so a
	/// future change cannot accidentally send the parameter twice and
	/// have Podman reject the request.
	#[tokio::test]
	async fn build_query_carries_rm_and_forcerm() {
		let (_dir, _fake, engine, _ctx, requests) = start_capture("proj");

		let file = crate::parse_str(
			"services:\n  app:\n    image: proj/img:1\n    build:\n      context: .\n",
		)
		.unwrap();
		engine
			.build_all_with_options(&file, &[], &crate::engine::BuildOptions::default())
			.await
			.expect("a build the fake accepts succeeds");

		let requests = requests.lock().unwrap().clone();
		let target = build_target(&requests);
		let query = target
			.split_once('?')
			.expect("the build target carries a query string");

		let rm_count = exact_param_count(query.1, "rm=true");
		assert_eq!(
			rm_count, 1,
			"the build query must carry `rm=true` exactly once, found {rm_count}: {query:?}"
		);
		let forcerm_count = exact_param_count(query.1, "forcerm=true");
		assert_eq!(
			forcerm_count, 1,
			"the build query must carry `forcerm=true` exactly once, found {forcerm_count}: {query:?}"
		);
	}

	/// `layers=true` must be present on the build query. The Docker
	/// compat handler used to default `layers` to true; the libpod
	/// handler defaults it to false. podup's user-facing behaviour
	/// is "the second build of the same Containerfile reuses the
	/// cache", which requires `layers=true`. Sent once, not twice
	/// (Podman rejects a duplicate param).
	#[tokio::test]
	async fn build_query_carries_layers_true() {
		let (_dir, _fake, engine, _ctx, requests) = start_capture("proj");

		let file = crate::parse_str(
			"services:\n  app:\n    image: proj/img:1\n    build:\n      context: .\n",
		)
		.unwrap();
		engine
			.build_all_with_options(&file, &[], &crate::engine::BuildOptions::default())
			.await
			.expect("a build the fake accepts succeeds");

		let requests = requests.lock().unwrap().clone();
		let target = build_target(&requests);
		let query = target
			.split_once('?')
			.expect("the build target carries a query string");

		let layers_count = exact_param_count(query.1, "layers=true");
		assert_eq!(
			layers_count, 1,
			"the build query must carry `layers=true` exactly once, found {layers_count}: {query:?}"
		);
	}

	/// The two labels are url-encoded into `key=value` form, just like every
	/// other value in this query string. A label value containing characters
	/// Podman's parser rejects when raw (`:`, `+`, `&`) must reach the
	/// server percent-encoded, so a project name with a colon survives the
	/// round-trip.
	#[tokio::test]
	async fn layer_label_values_are_url_encoded() {
		// A project name containing a colon and a `+` to exercise both
		// characters that Podman rejects when raw in a query value.
		let (_dir, _fake, engine, _ctx, requests) = start_capture("pr+oj:1");

		let file = crate::parse_str(
			"services:\n  app+svc:\n    image: proj/img:1\n    build:\n      context: .\n",
		)
		.unwrap();
		engine
			.build_all_with_options(&file, &[], &crate::engine::BuildOptions::default())
			.await
			.expect("a build the fake accepts succeeds");

		let requests = requests.lock().unwrap().clone();
		let target = build_target(&requests);
		// The raw query still contains the percent-escapes; the decoded
		// form matches the project's value.
		assert!(
			target.contains("layerLabel=podup.project%3Dpr%2Boj%3A1"),
			"the project name's colon and plus must be percent-encoded: {target}"
		);
		assert!(
			target.contains("layerLabel=podup.service%3Dapp%2Bsvc"),
			"the service name's plus must be percent-encoded: {target}"
		);
	}
}

#[cfg(not(unix))]
mod tests {}
