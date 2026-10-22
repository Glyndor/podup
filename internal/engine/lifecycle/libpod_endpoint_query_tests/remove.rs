use super::*;

/// `DELETE /containers/{}` must carry `volumes=` (not `v=`) when the
/// caller asked for anonymous-volume removal. The libpod handler
/// reads `volumes=`; the Docker compat handler reads `v=`, which the
/// libpod handler ignores, so a `down -v` against libpod without
/// `volumes=` reclaims nothing. This is the unit test for the
/// `container_rm_path` helper.
#[tokio::test]
async fn down_v_sends_volumes_query_param() {
	let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
	let req_clone = requests.clone();
	let fake = fake_podman::start_replying(move |method, target| {
		req_clone.lock().unwrap().push(format!("{method} {target}"));
		if method == "DELETE" && target.contains("/containers/") {
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
		.down_with_options(&file, true)
		.await
		.expect("a down the fake accepts succeeds");

	let del_req = {
		let requests = requests.lock().unwrap();
		requests
			.iter()
			.find(|r| r.starts_with("DELETE ") && r.contains("/containers/proj-web-1?"))
			.expect("a DELETE for the container was issued")
			.clone()
	};
	let query = del_req
		.split_once('?')
		.expect("the delete target carries a query string");
	assert!(
		query.1.contains("volumes=true"),
		"libpod delete must read `volumes=true`, not `v=true`: {del_req:?}"
	);
	assert!(
		!query.1.split('&').any(|pair| pair == "v=true"),
		"the docker `v=true` must not appear alongside `volumes=true` on libpod: {del_req:?}"
	);
	assert!(
		query.1.contains("force=true"),
		"`down` tears down a running container, so `force=true` must still be present: {del_req:?}"
	);
}
