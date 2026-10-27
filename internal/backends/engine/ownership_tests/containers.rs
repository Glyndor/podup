use super::*;

#[tokio::test]
async fn finding_5_create_refuses_unowned_container_without_delete() {
	for owner in [
		serde_json::json!({}),
		serde_json::json!({"podup.project":"other"}),
	] {
		let fake = fake_podman::start(move |method, target| {
			if method == "GET" && target.ends_with("/containers/manual/json") {
				(
					200,
					serde_json::json!({"Id":"container-id", "Config":{"Labels":owner}}).to_string(),
				)
			} else if method == "POST" && target.ends_with("/containers/create") {
				(
					409,
					r#"{"message":"container name manual already in use"}"#.into(),
				)
			} else {
				empty_lists(method, target)
			}
		});
		let file =
			crate::parse_str("services:\n  web:\n    image: alpine\n    container_name: manual\n")
				.unwrap();
		let err = engine(&fake)
			.create_and_start("manual", "web", &file.services["web"], &file, false)
			.await
			.expect_err("a container name collision must be refused")
			.to_string();
		assert!(err.contains("manual"), "{err}");
		assert_no_delete(&fake);
	}
}
