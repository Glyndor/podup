use super::*;

#[tokio::test]
async fn finding_2_refuses_unowned_pod_reuse_recreate_and_remove() {
	let file = crate::parse_str("services: {}\nx-podman-pod: true\n").unwrap();
	let hash = crate::engine::pod::pod_config_hash(&[], &file);
	for owner in [None, Some("other")] {
		for recorded in [hash.as_str(), "stale"] {
			let mut labels = serde_json::json!({"podup.pod-config-hash":recorded});
			if let Some(owner) = owner {
				labels["podup.project"] = owner.into();
			}
			let fake = fake_podman::start(move |method, target| {
				if method == "GET" && target.ends_with("/pods/proj/json") {
					(
						200,
						serde_json::json!({"Id":"pod-id", "Labels":labels}).to_string(),
					)
				} else {
					empty_lists(method, target)
				}
			});
			let e = engine(&fake);
			let err = e
				.ensure_pod(&file, &[])
				.await
				.expect_err("unowned pod must be refused")
				.to_string();
			assert!(
				err.contains("proj") && err.contains("podup.project"),
				"{err}"
			);
			let err = e
				.remove_pod()
				.await
				.expect_err("unowned pod cannot be removed")
				.to_string();
			assert!(
				err.contains("proj") && err.contains("podup.project"),
				"{err}"
			);
			assert_no_delete(&fake);
		}
	}
}

#[tokio::test]
async fn finding_3_label_down_removes_main_pod_before_networks() {
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/pods/json?") {
			(
				200,
				r#"[{"Id":"pod-id","Name":"proj","Labels":{"podup.project":"proj"}}]"#.into(),
			)
		} else {
			empty_lists(method, target)
		}
	});
	engine(&fake).down_by_label(false).await.unwrap();
	let seen = fake.requests.lock().unwrap();
	let pod = seen
		.iter()
		.position(|r| r.starts_with("DELETE ") && r.contains("/pods/"))
		.expect("the main project pod must be removed");
	let networks = seen
		.iter()
		.position(|r| r.contains("/networks/json?"))
		.unwrap();
	assert!(
		pod < networks,
		"pod infra container must release networks first: {seen:?}"
	);
}
