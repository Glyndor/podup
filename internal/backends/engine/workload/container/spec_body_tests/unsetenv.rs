use super::*;

/// The create body for a service whose `environment:` is `entries`, resolved
/// the way `up` resolves it.
async fn body_for(entries: &str) -> serde_json::Value {
	let fake = fake_routing();
	let engine = engine_for(&fake);
	let file = crate::compose::parse_str(&format!(
		"services:\n  web:\n    image: example/web:1\n    environment: {entries}\n"
	))
	.expect("compose must parse");
	let service = file.services.get("web").unwrap().clone();
	let base = tempfile::tempdir().unwrap();
	let env = crate::engine::container::effective_env(&service, base.path(), &[])
		.expect("the environment resolves");
	engine
		.create_and_start("proj-web-1", "web", &service, &file, true, &env)
		.await
		.expect("create_and_start must succeed against the fake socket");
	let bodies = fake.bodies.lock().unwrap().clone();
	decode_create_body(&bodies)
}

/// A variable the compose file names without a value, and nothing resolves,
/// must also leave the image's own value out. libpod does that for the names
/// in `unsetenv`, before it applies `env`.
#[tokio::test]
#[cfg(unix)]
async fn an_unresolved_key_is_sent_in_unsetenv_and_not_in_env() {
	let body = body_for("[P1_UNRESOLVED_KEY, KEPT=1]").await;
	assert_eq!(body["unsetenv"], serde_json::json!(["P1_UNRESOLVED_KEY"]));
	assert_eq!(body["env"], serde_json::json!({"KEPT": "1"}));
}

#[tokio::test]
#[cfg(unix)]
async fn a_fully_valued_environment_sends_no_unsetenv() {
	let body = body_for("[A=1, B=]").await;
	assert!(body.get("unsetenv").is_none(), "{body}");
}
