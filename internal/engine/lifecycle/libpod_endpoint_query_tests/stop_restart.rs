use super::*;

/// `POST /containers/{}/stop` must carry `timeout=` and not `t=`. The
/// libpod handler reads `timeout=`; the Docker compat handler reads
/// `t=`, so an unchanged `t=` query key leaves the libpod handler
/// ignoring the grace period entirely (defaulting to the container's
/// own stop timeout, or to the daemon default when there is none).
/// This is the unit test for the compensation in `commands/stop.rs::stop_container`.
#[tokio::test]
async fn stop_sends_timeout_query_param() {
	let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
	let req_clone = requests.clone();
	let fake = fake_podman::start_replying(move |method, target| {
		req_clone.lock().unwrap().push(format!("{method} {target}"));
		if method == "POST" && target.contains("/stop?") {
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

	// A service whose `stop_grace_period` is 7 seconds. The helper pins the
	// value the libpod side must see.
	let file =
		crate::parse_str("services:\n  web:\n    image: x\n    stop_grace_period: 7s\n").unwrap();
	engine
		.stop(&file, &["web".into()])
		.await
		.expect("a stop the fake accepts succeeds");

	let stop_req = {
		let requests = requests.lock().unwrap();
		requests
			.iter()
			.find(|r| r.starts_with("POST ") && r.contains("/stop?"))
			.expect("a /stop request was issued")
			.clone()
	};
	let query = stop_req
		.split_once('?')
		.expect("the stop target carries a query string");
	assert!(
		query.1.contains("timeout=7"),
		"the libpod stop endpoint must read `timeout=`, not `t=`: {stop_req:?}"
	);
	assert!(
		!query.1.split('&').any(|pair| pair.starts_with("t=")),
		"the docker `t=` must not appear alongside `timeout=` on libpod: {stop_req:?}"
	);
}

/// `POST /containers/{}/restart` must carry `timeout=` and not `t=`. The
/// libpod handler ignores `t=` and defaults `timeout=` to 0 when absent,
/// which would make every restart an immediate SIGKILL (a single-replica
/// `restart` on libpod would never give the container a chance to
/// drain). This is the unit test for `parallel/mod.rs::restart_one_service`
/// and the watch restart at `watch/mod.rs::watch_restart`.
#[tokio::test]
async fn restart_sends_timeout_query_param() {
	let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
	let req_clone = requests.clone();
	let fake = fake_podman::start_replying(move |method, target| {
		req_clone.lock().unwrap().push(format!("{method} {target}"));
		if method == "POST" && target.contains("/restart?") {
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

	let file =
		crate::parse_str("services:\n  web:\n    image: x\n    stop_grace_period: 5s\n").unwrap();
	engine
		.restart(&file, Some("web"))
		.await
		.expect("a restart the fake accepts succeeds");

	let restart_req = {
		let requests = requests.lock().unwrap();
		requests
			.iter()
			.find(|r| r.starts_with("POST ") && r.contains("/restart?"))
			.expect("a /restart request was issued")
			.clone()
	};
	let query = restart_req
		.split_once('?')
		.expect("the restart target carries a query string");
	assert!(
		query.1.contains("timeout="),
		"the libpod restart endpoint must read `timeout=`: {restart_req:?}"
	);
	assert!(
		!query.1.split('&').any(|pair| pair.starts_with("t=")),
		"the docker `t=` must not appear alongside `timeout=` on libpod: {restart_req:?}"
	);
}
