use super::*;

#[tokio::test]
async fn finding_4_cleanup_failures_propagate_and_continue() {
	for compose in [false, true] {
		// `down` with a compose file sweeps networks and secrets by label;
		// pods and volumes are swept by label only when there is no file.
		let swept: &[&str] = if compose {
			&["networks/json", "secrets/json"]
		} else {
			&["networks/json", "volumes/json", "pods/json", "secrets/json"]
		};
		for &failed in swept {
			let fake = fake_podman::start(move |method, target| {
				if method == "GET" && target.contains(failed) {
					(500, r#"{"message":"cleanup listing unavailable"}"#.into())
				} else {
					empty_lists(method, target)
				}
			});
			let e = engine(&fake);
			let result = if compose {
				e.down_with_options(&ComposeFile::default(), true).await
			} else {
				e.down_by_label(true).await
			};
			let err = result
				.expect_err("failed cleanup listing must propagate")
				.to_string();
			assert!(
				err.contains("cleanup listing unavailable"),
				"{failed}: {err}"
			);
			let seen = fake.requests.lock().unwrap();
			assert!(
				seen.iter().any(|r| r.contains("/secrets/json")),
				"cleanup must continue: {seen:?}"
			);
		}
	}
}

#[tokio::test]
async fn finding_4_cleanup_deletion_failures_propagate() {
	for resource in ["networks", "pods", "secrets"] {
		let fake = fake_podman::start(move |method, target| {
			if method == "DELETE" && target.contains(&format!("/{resource}/")) {
				(500, r#"{"message":"cleanup deletion unavailable"}"#.into())
			} else if method == "GET" && target.contains(&format!("/{resource}/json")) {
				let entry = match resource {
					"networks" => {
						serde_json::json!({"id":"net-id", "name":"old", "labels":{"podup.project":"proj"}})
					}
					"pods" => {
						serde_json::json!({"Id":"pod-id", "Name":"old", "Labels":{"podup.project":"proj"}})
					}
					_ => {
						serde_json::json!({"ID":"secret-id", "Spec":{"Name":"old", "Labels":{"podup.project":"proj"}}})
					}
				};
				(200, serde_json::json!([entry]).to_string())
			} else {
				empty_lists(method, target)
			}
		});
		// Pods are swept by label only when `down` has no compose file.
		let e = engine(&fake);
		let result = if resource == "pods" {
			e.down_by_label(false).await
		} else {
			e.down_with_options(&ComposeFile::default(), false).await
		};
		let err = result
			.expect_err("failed cleanup deletion must propagate")
			.to_string();
		assert!(
			err.contains("cleanup deletion unavailable"),
			"{resource}: {err}"
		);
	}
}
