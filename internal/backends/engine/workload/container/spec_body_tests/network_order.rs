use super::*;

/// The create body for a service on three networks with the given priorities.
async fn body_for(priorities: [&str; 3]) -> serde_json::Value {
	let fake = fake_routing();
	let engine = engine_for(&fake);
	let [a, b, c] = priorities;
	let file = crate::compose::parse_str(&format!(
		"services:\n  web:\n    image: example/web:1\n    networks:\n      front:\n{a}      back:\n{b}      mid:\n{c}networks:\n  front: {{}}\n  back: {{}}\n  mid: {{}}\n"
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

/// Compose asks for the highest priority first, and libpod connects the
/// networks in the order of `networkOrder`, the first being `eth0`.
#[tokio::test]
#[cfg(unix)]
async fn priority_becomes_the_network_order() {
	let body = body_for([
		"        priority: 10\n",
		"        priority: 1000\n",
		"        priority: 10\n",
	])
	.await;
	assert_eq!(
		body["networkOrder"],
		serde_json::json!(["proj_back", "proj_front", "proj_mid"])
	);
	assert!(
		body["networks"]["proj_back"].get("priority").is_none(),
		"priority is not a libpod field: {body}"
	);
}

/// With no priority libpod orders by name, as it always did, so nothing is sent.
#[tokio::test]
#[cfg(unix)]
async fn no_priority_sends_no_order() {
	let body = body_for(["        {}\n", "        {}\n", "        {}\n"]).await;
	assert!(body.get("networkOrder").is_none(), "{body}");
}
