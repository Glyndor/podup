use super::*;

#[tokio::test]
async fn finding_15_down_refuses_unlabelled_network() {
	let file = crate::parse_str("services: {}\nnetworks:\n  private:\n    name: manual\n").unwrap();
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.ends_with("/networks/manual/json") {
			(200, r#"{"id":"net-id","name":"manual","labels":{}}"#.into())
		} else {
			empty_lists(method, target)
		}
	});
	let err = engine(&fake)
		.down(&file)
		.await
		.expect_err("an unlabelled network must be refused")
		.to_string();
	assert!(
		err.contains("manual") && err.contains("podup.project") && err.contains("external"),
		"{err}"
	);
	assert_no_delete(&fake);
}
