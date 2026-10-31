use super::*;

/// `POST /containers/{}/kill?signal=SIGKILL` must be followed by a
/// `POST /containers/{}/wait?condition=stopped` on libpod. The Docker
/// compat `/kill` handler blocks on the container when the signal is
/// SIGKILL/9/KILL/0; the libpod handler replies immediately. Without
/// the follow-up wait, a caller that relied on the compat handler's
/// semantics would observe a still-running container when `kill`
/// returned. SIGTERM and the other graceful signals do not block
/// either way and the wait is skipped.
#[tokio::test]
async fn kill_with_sigkill_sends_follow_up_wait() {
	let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
	let req_clone = requests.clone();
	let fake = fake_podman::start_replying(move |method, target| {
		req_clone.lock().unwrap().push(format!("{method} {target}"));
		if method == "POST" && target.contains("/kill?") {
			FakeReply::Body(204, String::new())
		} else if method == "POST" && target.contains("/wait?") {
			// The wait endpoint returns an HTTP 200 with the exit code as
			// its body; a plain `Body(200, "0")` is the libpod shape.
			FakeReply::Body(200, "0".to_string())
		} else if method == "GET" && target.contains("/containers/json") {
			FakeReply::Body(
				200,
				r#"[{"Names":["/proj-web-1"],"State":"running","Labels":{"podup.service":"web"}}]"#
					.into(),
			)
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.into())
		}
	});
	let engine = engine_with(fake.client(), "proj");

	let file = crate::parse_str("services:\n  web:\n    image: x\n").unwrap();
	engine
		.kill(&file, &[], "SIGKILL")
		.await
		.expect("a kill the fake accepts succeeds");

	let requests = requests.lock().unwrap();
	let kill_req = requests
		.iter()
		.find(|r| r.starts_with("POST ") && r.contains("/kill?"))
		.expect("a /kill request was issued");
	assert!(
		kill_req.contains("signal=SIGKILL"),
		"the kill must carry the requested signal: {kill_req:?}"
	);
	let wait_req = requests
		.iter()
		.find(|r| r.starts_with("POST ") && r.contains("/wait?"))
		.expect("a /wait?condition=stopped follow-up must follow the kill");
	assert!(
		wait_req.contains("condition=stopped"),
		"the follow-up wait must pin the container's stopped state: {wait_req:?}"
	);
}

/// The opposite case: SIGTERM is graceful and the libpod handler
/// replies promptly anyway, so a `kill -s SIGTERM <id>` must NOT pin
/// the caller behind a per-container `/wait?condition=stopped`. The
/// compensation's contract is that the wait fires for SIGKILL/9/KILL/0
/// only; adding it for SIGTERM is what would turn a 50-replica `kill`
/// into 50 sequential waits.
#[tokio::test]
async fn kill_with_sigterm_skips_the_follow_up_wait() {
	let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
	let req_clone = requests.clone();
	let fake = fake_podman::start_replying(move |method, target| {
		req_clone.lock().unwrap().push(format!("{method} {target}"));
		if method == "POST" && target.contains("/kill?") {
			FakeReply::Body(204, String::new())
		} else if method == "GET" && target.contains("/containers/json") {
			FakeReply::Body(
				200,
				r#"[{"Names":["/proj-web-1"],"State":"running","Labels":{"podup.service":"web"}}]"#
					.into(),
			)
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.into())
		}
	});
	let engine = engine_with(fake.client(), "proj");

	let file = crate::parse_str("services:\n  web:\n    image: x\n").unwrap();
	engine
		.kill(&file, &[], "SIGTERM")
		.await
		.expect("a kill the fake accepts succeeds");

	let requests = requests.lock().unwrap();
	assert!(
		requests
			.iter()
			.any(|r| r.starts_with("POST ") && r.contains("/kill?signal=SIGTERM")),
		"a /kill?signal=SIGTERM must be issued: {requests:?}"
	);
	assert!(
		!requests
			.iter()
			.any(|r| r.starts_with("POST ") && r.contains("/wait?")),
		"SIGTERM must not trigger a follow-up /wait?condition=stopped: {requests:?}"
	);
}
