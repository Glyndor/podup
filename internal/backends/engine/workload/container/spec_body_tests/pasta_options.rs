use super::*;

/// `network_mode: "pasta:-T,15432"` must reach libpod as the bare `pasta`
/// mode plus its options, the split `podman run --network pasta:...` does.
/// Sent whole, Podman rejected it as `invalid network "pasta:-T,15432"`
/// (#1994).
#[tokio::test]
#[cfg(unix)]
async fn pasta_options_are_sent_apart_from_the_mode() {
	let fake = fake_routing();
	let engine = engine_for(&fake);
	let file = crate::compose::parse_str(
		"services:\n  web:\n    image: example/web:1\n    network_mode: \"pasta:-T,15432\"\n",
	)
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
	let body = decode_create_body(&fake.bodies.lock().unwrap().clone());
	assert_eq!(body["netns"], serde_json::json!({"nsmode": "pasta"}));
	assert_eq!(
		body["network_options"],
		serde_json::json!({"pasta": ["-T", "15432"]})
	);
}
