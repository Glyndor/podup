use super::*;

async fn mounts(extra: &str) -> serde_json::Value {
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
		.map(|_| ())
		.map_err(|e| e.to_string())
		.expect("create_and_start must succeed against the fake socket");
	let bodies = fake.bodies.lock().unwrap().clone();
	decode_create_body(&bodies)["mounts"].clone()
}

#[tokio::test]
#[cfg(unix)]
async fn a_tmpfs_owner_that_is_the_user_is_sent_as_u() {
	let m = mounts("    user: \"1000:1000\"\n    tmpfs:\n      - /t:uid=1000,gid=1000,mode=1777\n")
		.await;
	assert_eq!(m[0]["destination"], "/t", "{m}");
	assert_eq!(
		m[0]["options"],
		serde_json::json!(["mode=1777", "U"]),
		"{m}"
	);
}

#[tokio::test]
#[cfg(unix)]
async fn a_tmpfs_without_an_owner_is_sent_as_written() {
	let m = mounts("    tmpfs:\n      - /t:size=64m,mode=1777\n").await;
	assert_eq!(
		m[0]["options"],
		serde_json::json!(["size=64m", "mode=1777"]),
		"{m}"
	);
}

#[tokio::test]
#[cfg(unix)]
async fn a_tmpfs_owner_that_is_not_the_user_never_reaches_the_create() {
	let fake = fake_routing();
	let engine = engine_for(&fake);
	let file = crate::compose::parse_str(
		"services:\n  web:\n    image: example/web:1\n    tmpfs:\n      - /t:uid=1000\n",
	)
	.unwrap();
	let service = file.services.get("web").unwrap().clone();
	let err = engine
		.create_and_start(
			"proj-web-1",
			"web",
			&service,
			&file,
			true,
			&std::collections::BTreeMap::new(),
		)
		.await
		.map(|_| ())
		.expect_err("no user: to give the tmpfs to");
	assert!(err.to_string().contains("Podman has no uid/gid"), "{err}");
	assert!(
		fake.bodies.lock().unwrap().iter().all(|b| b.is_empty()),
		"something was created"
	);
}
