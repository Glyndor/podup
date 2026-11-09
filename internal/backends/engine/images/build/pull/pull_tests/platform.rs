#[cfg(unix)]
use crate::engine::fake_podman;

/// The pull endpoint reads the platform as `os`, `arch` and `variant`; a
/// `platform` parameter is ignored and the host's architecture comes down.
#[tokio::test]
#[cfg(unix)]
async fn pull_selects_the_service_platform_with_the_parameters_the_endpoint_reads() {
	let fake = fake_podman::start(|method, target| {
		if method == "POST" && target.contains("/images/pull") {
			(200, String::new())
		} else if method == "GET" && target.contains("/images/") && target.contains("/json") {
			(200, "{}".to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = crate::engine::Engine::new(fake.client(), "proj".into());
	let file = crate::parse_str("services:\n  a:\n    image: shared\n    platform: linux/arm/v7\n")
		.unwrap();

	e.pull_services(&file, &[])
		.await
		.expect("a pull the fake accepts succeeds");

	let seen = fake.requests.lock().unwrap();
	let pull = seen
		.iter()
		.find(|r| r.contains("/images/pull"))
		.expect("a pull request was issued");
	assert!(
		pull.contains("&os=linux&arch=arm&variant=v7"),
		"the pull must carry os, arch and variant: {pull}"
	);
	assert!(
		!pull.contains("platform="),
		"the pull endpoint ignores a platform parameter: {pull}"
	);
}
