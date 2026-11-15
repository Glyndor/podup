use super::*;

#[test]
fn a_rootless_info_document_reports_rootless() {
	let info: Info = serde_json::from_str(r#"{"host":{"security":{"rootless":true}}}"#).unwrap();
	assert!(info.is_rootless());
}

#[test]
fn a_rootful_or_incomplete_info_document_does_not() {
	for doc in [
		r#"{"host":{"security":{"rootless":false}}}"#,
		r#"{"host":{}}"#,
		"{}",
	] {
		let info: Info = serde_json::from_str(doc).unwrap();
		assert!(!info.is_rootless(), "{doc}");
	}
}

/// Create body of a one-service `up` against a fake daemon whose `/info`
/// answers with `info`.
#[cfg(unix)]
async fn create_body_with_info(info: &'static str, devices: &str) -> serde_json::Value {
	let fake = crate::engine::fake_podman::start(move |method, target| {
		if method == "GET" && target.ends_with("/info") {
			(200, info.to_string())
		} else if method == "GET" && target.contains("/containers/json") {
			(200, "[]".to_string())
		} else if method == "POST" && target.contains("/images/pull") {
			(200, String::new())
		} else if method == "POST" && target.contains("/containers/create") {
			(200, "{}".to_string())
		} else if method == "POST" && target.contains("/start") {
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	let file = crate::parse_str(&format!(
		"services:\n  web:\n    image: img\n    devices: [{devices}]\n"
	))
	.unwrap();
	e.up_with_options(&file, false, &[], &[], false, false, false, false)
		.await
		.expect("up must succeed");
	let reqs = fake.requests.lock().unwrap();
	let bodies = fake.bodies.lock().unwrap();
	let body = reqs
		.iter()
		.zip(bodies.iter())
		.find(|(r, _)| r.contains("/containers/create"))
		.map(|(_, b)| b)
		.expect("a create request");
	serde_json::from_slice(body).unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn rootless_daemon_gets_the_device_without_a_cgroup_rule() {
	let info = r#"{"host":{"security":{"rootless":true}}}"#;
	for perm in ["r", "rw", "rwm", "mrw"] {
		let body = create_body_with_info(info, &format!("\"/dev/null:/dev/null:{perm}\"")).await;
		assert_eq!(body["devices"].as_array().map(Vec::len), Some(1), "{perm}");
		let rules = body.get("device_cgroup_rule");
		assert!(
			rules.is_none_or(|r| r.as_array().is_some_and(Vec::is_empty)),
			"{perm}: {rules:?}"
		);
	}
}

#[cfg(unix)]
#[tokio::test]
async fn rootful_daemon_still_gets_the_permission_rule() {
	let info = r#"{"host":{"security":{"rootless":false}}}"#;
	let body = create_body_with_info(info, "\"/dev/null:/dev/null:r\"").await;
	assert_eq!(body["device_cgroup_rule"][0]["access"], "r");
}
