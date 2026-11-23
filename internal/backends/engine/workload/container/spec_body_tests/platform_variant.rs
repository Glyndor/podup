use super::*;

/// The create body a service's `platform:` produces, by the fields that choose
/// the instance of a manifest list.
async fn platform_fields(platform: &str) -> serde_json::Value {
	let fake = fake_routing();
	let engine = engine_for(&fake);
	let file = crate::compose::parse_str(&format!(
		"services:\n  web:\n    image: example/web:1\n    platform: {platform}\n"
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

/// The pull already sent the variant. Podman's create picks the instance of a
/// manifest list by OS, architecture and variant, so leaving it out there
/// starts the instance that suits the host instead of the one the file names.
#[tokio::test]
#[cfg(unix)]
async fn the_variant_of_a_platform_reaches_the_create_request() {
	let body = platform_fields("linux/arm/v7").await;
	assert_eq!(body["image_os"], "linux");
	assert_eq!(body["image_arch"], "arm");
	assert_eq!(body["image_variant"], "v7");
}

#[tokio::test]
#[cfg(unix)]
async fn a_platform_without_a_variant_sends_none() {
	let body = platform_fields("linux/arm64").await;
	assert_eq!(body["image_arch"], "arm64");
	assert!(body.get("image_variant").is_none(), "{body}");
}

#[tokio::test]
#[cfg(unix)]
async fn a_bare_architecture_sends_no_os_and_no_variant() {
	let body = platform_fields("arm64").await;
	assert_eq!(body["image_arch"], "arm64");
	assert!(body.get("image_os").is_none(), "{body}");
	assert!(body.get("image_variant").is_none(), "{body}");
}
