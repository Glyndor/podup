use super::*;

/// What the create call carries for `web` when its compose says `extra` under it.
async fn create_body(extra: &str) -> serde_json::Value {
	let fake = fake_routing();
	let engine = engine_for(&fake);
	let file = crate::compose::parse_str(&format!(
		"services:\n  web:\n    image: example/web:1\n{extra}"
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
	decode_create_body(&bodies)
}

fn is_empty_list(v: &serde_json::Value) -> bool {
	v == &serde_json::json!([])
}

/// Compose Specification, `entrypoint`: "If the value is `[]` (empty list) or
/// `''` (empty string), the default entrypoint declared by the image is ignored,
/// i.e. overridden to be empty." Docker Compose treats `command: ""` the same:
/// the image's command runs. `sh -c ""` is a shell that exits at once.
#[tokio::test]
#[cfg(unix)]
async fn an_empty_string_entrypoint_is_the_empty_list_and_not_an_empty_shell() {
	let body = create_body("    entrypoint: \"\"\n").await;
	assert!(is_empty_list(&body["entrypoint"]), "{body}");
	let list = create_body("    entrypoint: []\n").await;
	assert_eq!(body["entrypoint"], list["entrypoint"], "the control, []");
}

#[tokio::test]
#[cfg(unix)]
async fn an_empty_string_command_is_the_empty_list_and_not_an_empty_shell() {
	let body = create_body("    command: \"\"\n").await;
	assert!(is_empty_list(&body["command"]), "{body}");
	let blanks = create_body("    command: \"   \"\n").await;
	assert!(is_empty_list(&blanks["command"]), "{blanks}");
}

#[tokio::test]
#[cfg(unix)]
async fn a_command_with_words_still_runs_through_sh_c() {
	let body = create_body("    command: echo hi\n").await;
	assert_eq!(body["command"], serde_json::json!(["sh", "-c", "echo hi"]));
}
