use super::*;

/// What the create call of a service named `web` carries once the compose has
/// been parsed and `labels` (extra `KEY=VALUE` lines) added to its copy.
async fn create_body(extra_label: Option<&str>) -> serde_json::Value {
	let fake = fake_routing();
	let engine = engine_for(&fake);
	let file = crate::compose::parse_str(
		"services:\n  web:\n    image: example/web:1\n    networks:\n      back:\n        aliases: [api]\nnetworks:\n  back: {}\n",
	)
	.expect("compose must parse");
	let mut service = file.services.get("web").unwrap().clone();
	if let Some(label) = extra_label {
		service.labels = crate::compose::types::Labels::List(vec![label.to_string()]);
	}
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

/// A container `run` makes does not answer to the service name or to the
/// service's aliases; the same service started by `up` does.
#[tokio::test]
#[cfg(unix)]
async fn a_one_off_is_not_reachable_under_the_service_aliases() {
	let service = create_body(None).await;
	let aliases = &service["networks"]["proj_back"]["aliases"];
	assert_eq!(
		aliases,
		&serde_json::json!(["web", "api"]),
		"control: {service}"
	);

	let one_off = create_body(Some("podup.oneoff=true")).await;
	let aliases = &one_off["networks"]["proj_back"]["aliases"];
	assert!(
		aliases.is_null() || aliases == &serde_json::json!([]),
		"a one-off kept aliases: {one_off}"
	);
}
