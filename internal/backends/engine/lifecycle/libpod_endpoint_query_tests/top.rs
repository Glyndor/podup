use super::*;

/// `GET /containers/{}/top` must carry `ps_args=-ef`. The libpod handler
/// defaults `ps_args` to its own descriptors (a different column
/// set), so podup's user-facing `top` output would drift the moment
/// a service moved off a default-configured image.
#[tokio::test]
async fn top_sends_ps_args_query_param() {
	let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
	let req_clone = requests.clone();
	let fake = fake_podman::start_replying(move |method, target| {
		req_clone.lock().unwrap().push(format!("{method} {target}"));
		if method == "GET" && target.contains("/top?") {
			FakeReply::Body(
				200,
				r#"{"Titles":["UID","PID","PPID","C","STIME","TTY","TIME","CMD"],"Processes":[["root","1","0","0","15:00","?","00:00:00","sleep 3600"]]}"#
					.into(),
			)
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
		.top_with_options(&file, &[], false)
		.await
		.expect("a top the fake accepts succeeds");

	let top_req = {
		let requests = requests.lock().unwrap();
		requests
			.iter()
			.find(|r| r.starts_with("GET ") && r.contains("/top?"))
			.expect("a /top request was issued")
			.clone()
	};
	let query = top_req
		.split_once('?')
		.expect("the top target carries a query string");
	assert!(
		query.1.contains("ps_args=-ef"),
		"the libpod top endpoint must read `ps_args=-ef`: {top_req:?}"
	);
}
