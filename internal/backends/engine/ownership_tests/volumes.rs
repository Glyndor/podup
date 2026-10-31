use super::*;

fn volume_file(external: bool) -> ComposeFile {
	crate::parse_str(&format!(
		"services: {{}}\nvolumes:\n  data:\n    name: shared\n    external: {external}\n"
	))
	.unwrap()
}

#[tokio::test]
async fn finding_1_down_refuses_unowned_volume() {
	for labels in [
		serde_json::json!({}),
		serde_json::json!({"podup.project":"other"}),
	] {
		let fake = fake_podman::start(move |method, target| {
			if method == "GET" && target.ends_with("/volumes/shared/json") {
				(
					200,
					serde_json::json!({"Name":"shared", "Labels":labels}).to_string(),
				)
			} else {
				empty_lists(method, target)
			}
		});
		let err = engine(&fake)
			.down_with_options(&volume_file(false), true)
			.await
			.expect_err("an unowned volume must be refused")
			.to_string();
		assert!(
			err.contains("shared") && err.contains("podup.project") && err.contains("external"),
			"{err}"
		);
		assert_no_delete(&fake);
	}
}

#[tokio::test]
async fn finding_1_create_refuses_unowned_volume_conflict() {
	for labels in [
		serde_json::json!({}),
		serde_json::json!({"podup.project":"other"}),
	] {
		let fake = fake_podman::start(move |method, target| {
			if method == "POST" && target.ends_with("/volumes/create") {
				(500, r#"{"message":"volume already exists"}"#.into())
			} else if method == "GET" && target.ends_with("/volumes/shared/json") {
				(
					200,
					serde_json::json!({"Name":"shared", "Labels":labels}).to_string(),
				)
			} else {
				empty_lists(method, target)
			}
		});
		let err = engine(&fake)
			.create_volumes(&volume_file(false))
			.await
			.expect_err("an unowned volume cannot be reused")
			.to_string();
		assert!(
			err.contains("shared") && err.contains("podup.project") && err.contains("external"),
			"{err}"
		);
		assert_no_delete(&fake);
	}
}
