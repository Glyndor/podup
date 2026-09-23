//! Tests for the cgroup_manager hint appended to a build error (#1778).
//!
//! Four cases pin the hint to the condition that triggers it, not to the
//! wording of a runtime message:
//!
//! - a build failure with `host.cgroupManager = "systemd"` carries the hint,
//! - a build failure with `host.cgroupManager = "cgroupfs"` does NOT,
//! - a build failure where the info call fails carries the original error
//!   and no hint,
//! - a SUCCESSFUL build never calls info at all.
//!
//! The fourth case needs the call to be observable. Each test wires a fake
//! libpod that records every request and inspects the request log, not the
//! source.

#[cfg(unix)]
mod tests {
	use std::sync::{Arc, Mutex};

	use crate::engine::fake_podman::{self, FakeReply};
	use crate::engine::Engine;

	/// Compose file the build pass uses: one service with a `build:` and an
	/// `image:` so the build path reaches the libpod POST.
	const FILE: &str = "\
services:
  app:
    image: proj/img:1
    build:
      context: .
";

	/// Build-context directory with a real Dockerfile so `build_service`'s
	/// `fs::metadata` check passes.
	fn context() -> (tempfile::TempDir, std::path::PathBuf) {
		let dir = tempfile::tempdir().expect("tempdir");
		let path = dir.path().to_path_buf();
		std::fs::write(
			path.join("Dockerfile"),
			b"FROM docker.io/library/alpine:3.20\nRUN echo hi\nCMD [\"echo\",\"hi\"]\n",
		)
		.expect("write Dockerfile");
		(dir, path)
	}

	/// The body the libpod build endpoint emits when the build fails with
	/// the in-band error. Two STEP lines followed by a fatal `error` line.
	const FAIL_BODY: &str = "{\"stream\":\"STEP 1/3: FROM docker.io/library/alpine:3.20\\n\"}\n\
		{\"stream\":\"--> 3f3c8b769775\\n\"}\n\
		{\"stream\":\"STEP 2/3: RUN false\\n\"}\n\
		{\"error\":\"The command '/bin/sh -c false' returned a non-zero code: 1\\n\"}\n";

	/// A success-body: a single `--> sha256:...` image id followed by
	/// `Successfully tagged`. The compose file's `image:` triggers a
	/// follow-up `POST /libpod/images/{tag}/tag` which the routing
	/// closure answers with 200.
	const SUCCESS_BODY: &str = "\
		{\"stream\":\"--> sha256:1111111111111111111111111111111111111111111111111111111111111111\\n\"}\n\
		{\"stream\":\"Successfully tagged proj/img:1\\n\"}\n";

	/// Stand up a fake Podman and engine bound to the same request log.
	/// Routes every request to a default 404 except:
	/// - `POST /libpod/build?`: returns `body_status` and `body`,
	/// - `POST /libpod/images/.../tag`: 200 with no body,
	/// - `GET /libpod/info`: returns `info_status` and `info_body`.
	fn start_fake(
		body_status: u16,
		body: &'static str,
		info_status: u16,
		info_body: &'static str,
		context: std::path::PathBuf,
	) -> (fake_podman::FakePodman, Engine, Arc<Mutex<Vec<String>>>) {
		let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
		let closure_requests = requests.clone();
		let fake = fake_podman::start_replying(move |method, target| {
			closure_requests
				.lock()
				.unwrap()
				.push(format!("{method} {target}"));
			if method == "POST" && target.contains("/build?") {
				FakeReply::Body(body_status, body.to_string())
			} else if method == "POST" && target.contains("/images/") && target.contains("/tag") {
				FakeReply::Body(200, String::new())
			} else if method == "GET" && target.ends_with("/info") {
				FakeReply::Body(info_status, info_body.to_string())
			} else {
				FakeReply::Body(404, r#"{"message":"not found"}"#.to_string())
			}
		});
		let engine = Engine::with_base_dir(fake.client(), "proj".into(), context);
		(fake, engine, requests)
	}

	/// Substring used by every hint-affirming assertion: the key/value the
	/// hint names. Kept in one place so the four tests pin the same shape.
	const HINT_MARKER: &str = "cgroup_manager = \"cgroupfs\"";

	/// Build fails, info says `systemd` → hint appended.
	#[tokio::test]
	async fn failed_build_appends_hint_when_cgroup_manager_is_systemd() {
		let (_dir, ctx_path) = context();
		let (_fake, engine, _requests) = start_fake(
			200,
			FAIL_BODY,
			200,
			r#"{"host":{"cgroupManager":"systemd"}}"#,
			ctx_path,
		);
		let file = crate::parse_str(FILE).expect("the fixture parses");

		let err = engine
			.build_all_with_options(&file, &[], &crate::engine::BuildOptions::default())
			.await
			.expect_err("a build whose last line is `error` must fail the pass");
		let msg = err.to_string();
		assert!(
			msg.contains("returned a non-zero code"),
			"the original runtime error reaches the caller: {msg}"
		);
		assert!(
			msg.contains(HINT_MARKER),
			"the hint names cgroup_manager = \"cgroupfs\": {msg}"
		);
		assert!(
			msg.contains("~/.config/containers/containers.conf"),
			"the hint names the file the key goes in: {msg}"
		);
		assert!(
			msg.contains("podman-machine"),
			"the hint names the WSL/podman-machine condition: {msg}"
		);
		assert!(
			!msg.contains("\n\nhint:"),
			"buildah's error string already terminates with a newline; the separator must not introduce a blank line before the hint: {msg}"
		);
		assert!(
			msg.contains("\nhint:"),
			"the hint sits on its own line directly after the error: {msg}"
		);
	}

	/// Build fails, info says `cgroupfs` → no hint.
	#[tokio::test]
	async fn failed_build_does_not_hint_when_cgroup_manager_is_cgroupfs() {
		let (_dir, ctx_path) = context();
		let (_fake, engine, _requests) = start_fake(
			200,
			FAIL_BODY,
			200,
			r#"{"host":{"cgroupManager":"cgroupfs"}}"#,
			ctx_path,
		);
		let file = crate::parse_str(FILE).expect("the fixture parses");

		let err = engine
			.build_all_with_options(&file, &[], &crate::engine::BuildOptions::default())
			.await
			.expect_err("a build whose last line is `error` must fail the pass");
		let msg = err.to_string();
		assert!(
			msg.contains("returned a non-zero code"),
			"the original runtime error reaches the caller: {msg}"
		);
		assert!(
			!msg.contains(HINT_MARKER),
			"a non-systemd manager must not trigger the hint: {msg}"
		);
		assert!(
			!msg.contains("hint:"),
			"no hint line is appended when the manager is not systemd: {msg}"
		);
	}

	/// Build fails, the info call itself errors → original message survives,
	/// no hint.
	#[tokio::test]
	async fn failed_build_keeps_original_error_when_info_call_fails() {
		let (_dir, ctx_path) = context();
		let (_fake, engine, _requests) = start_fake(
			200,
			FAIL_BODY,
			500,
			r#"{"message":"daemon overloaded"}"#,
			ctx_path,
		);
		let file = crate::parse_str(FILE).expect("the fixture parses");

		let err = engine
			.build_all_with_options(&file, &[], &crate::engine::BuildOptions::default())
			.await
			.expect_err("a build whose last line is `error` must fail the pass");
		let msg = err.to_string();
		assert!(
			msg.contains("returned a non-zero code"),
			"the original runtime error reaches the caller unchanged: {msg}"
		);
		assert!(
			!msg.contains(HINT_MARKER),
			"the hint must not appear when the info call failed: {msg}"
		);
		assert!(
			!msg.contains("hint:"),
			"no hint of any kind may be appended when the info call failed: {msg}"
		);
	}

	/// Build SUCCEEDS → no `/info` request is issued. Observed via the
	/// captured request log, not by reading the source.
	#[tokio::test]
	async fn successful_build_does_not_call_info() {
		let (_dir, ctx_path) = context();
		let (_fake, engine, requests) = start_fake(
			200,
			SUCCESS_BODY,
			200,
			r#"{"host":{"cgroupManager":"systemd"}}"#,
			ctx_path,
		);
		let file = crate::parse_str(FILE).expect("the fixture parses");

		engine
			.build_all_with_options(&file, &[], &crate::engine::BuildOptions::default())
			.await
			.expect("a build the fake accepts succeeds");

		let log = requests.lock().unwrap().clone();
		let info_calls: Vec<&String> = log
			.iter()
			.filter(|r| r.starts_with("GET ") && r.ends_with("/info"))
			.collect();
		assert!(
			info_calls.is_empty(),
			"a successful build must not call /info: {log:?}"
		);
	}
}

#[cfg(not(unix))]
mod tests {}
