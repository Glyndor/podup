use super::*;

/// libpod's answer for `GET /images/{name}/json` of an image built with
/// `HEALTHCHECK --interval=1m --timeout=5s --retries=7 --start-period=3s CMD /bin/true`
/// (shape measured on Podman 5.7.0: `Healthcheck` is a top-level key).
const IMAGE_WITH_HEALTHCHECK: &str = r#"{"Id":"sha256:abc","Healthcheck":{"Test":["CMD-SHELL","/bin/true"],"StartPeriod":3000000000,"Interval":60000000000,"Timeout":5000000000,"Retries":7}}"#;

async fn healthconfig(image_answer: Option<&str>, extra: &str) -> serde_json::Value {
	let answer = image_answer.map(str::to_string);
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, "[]".to_string())
		} else if method == "GET" && target.contains("/images/") && target.ends_with("/json") {
			match &answer {
				Some(body) => (200, body.clone()),
				None => (404, r#"{"message":"no such image"}"#.to_string()),
			}
		} else if method == "POST" && target.contains("/containers/create") {
			(200, r#"{"Id":"abc","Warnings":[]}"#.to_string())
		} else if method == "POST" && target.contains("/start") {
			(200, String::new())
		} else if method == "DELETE" && target.contains("/containers/") {
			(404, r#"{"message":"no such container"}"#.to_string())
		} else {
			(404, r#"{"message":"unexpected request"}"#.to_string())
		}
	});
	let engine = engine_for(&fake);
	let file = crate::compose::parse_str(&format!(
		"services:\n  web:\n    image: example/web:1\n    healthcheck:\n{extra}"
	))
	.expect("compose must parse");
	let service = file.services.get("web").unwrap().clone();
	engine
		.create_and_start(
			"proj-web-1",
			"web",
			&service,
			&file,
			true,
			&std::collections::BTreeMap::new(),
		)
		.await
		.expect("create_and_start must succeed against the fake socket");
	let bodies = fake.bodies.lock().unwrap().clone();
	decode_create_body(&bodies)["healthconfig"].clone()
}

#[tokio::test]
#[cfg(unix)]
async fn a_test_alone_keeps_the_timings_the_image_declares() {
	let hc = healthconfig(
		Some(IMAGE_WITH_HEALTHCHECK),
		"      test: [\"CMD\", \"true\"]\n",
	)
	.await;
	assert_eq!(hc["Test"], serde_json::json!(["CMD", "true"]), "{hc}");
	assert_eq!(hc["Interval"], 60_000_000_000_i64, "{hc}");
	assert_eq!(hc["Timeout"], 5_000_000_000_i64, "{hc}");
	assert_eq!(hc["Retries"], 7, "{hc}");
	assert_eq!(hc["StartPeriod"], 3_000_000_000_i64, "{hc}");
}

#[tokio::test]
#[cfg(unix)]
async fn an_image_that_cannot_be_read_leaves_the_specification_defaults() {
	let hc = healthconfig(None, "      test: [\"CMD\", \"true\"]\n").await;
	assert_eq!(hc["Interval"], 30_000_000_000_i64, "{hc}");
	assert_eq!(hc["Timeout"], 30_000_000_000_i64, "{hc}");
	assert_eq!(hc["Retries"], 3, "{hc}");
}

#[tokio::test]
#[cfg(unix)]
async fn an_image_with_a_null_healthcheck_leaves_the_specification_defaults() {
	let hc = healthconfig(
		Some(r#"{"Id":"sha256:abc","Healthcheck":null}"#),
		"      test: [\"CMD\", \"true\"]\n",
	)
	.await;
	assert_eq!(hc["Interval"], 30_000_000_000_i64, "{hc}");
}
