use super::*;
use std::cell::Cell;

#[tokio::test]
async fn f8_wait_errors_name_service_and_still_stop() {
	let errors = [
		PodmanError::Api {
			status: 500,
			message: "wait failed".into(),
		},
		PodmanError::Json(serde_json::from_str::<i64>("invalid").unwrap_err()),
		PodmanError::Connect(std::io::Error::from(std::io::ErrorKind::ConnectionReset)),
	];
	for error in errors {
		let stopped = Cell::new(false);
		let result = finish_abort(
			async { exit_code_result("web", "proj-web-2", Err(error)) },
			async {
				stopped.set(true);
				Ok(())
			},
		)
		.await;
		assert!(stopped.get(), "status errors must not bypass cleanup");
		let error = result.expect_err("unknown status must fail").to_string();
		assert!(error.contains("service web"), "{error}");
		assert!(error.contains("proj-web-2"), "{error}");
	}
}

#[tokio::test]
async fn f8_wait_zero_preserves_success_after_stop() {
	let stopped = Cell::new(false);
	let result = finish_abort(
		async { exit_code_result("web", "proj-web-1", Ok(0)) },
		async {
			stopped.set(true);
			Ok(())
		},
	)
	.await;
	assert_eq!(result.unwrap(), 0);
	assert!(stopped.get());
}

#[tokio::test]
async fn f8_trigger_replica_wins_with_distinct_codes() {
	let names = vec!["proj-web-1".into(), "proj-web-2".into()];
	for (trigger, expected) in [("proj-web-2", 7), ("proj-web-1", 137)] {
		let selected = select_exit_container("web", "web", trigger, &names, |_| async {
			panic!("the trigger already identifies the exited replica");
		})
		.await
		.unwrap();
		let code = if selected == names[1] { 7 } else { 137 };
		assert_eq!(code, expected, "selected {selected}");
	}
}

#[tokio::test]
async fn f8_other_service_selects_stopped_replica_or_first() {
	let names = vec!["proj-web-1".into(), "proj-web-2".into()];
	for stopped in [Some("proj-web-2"), Some("proj-web-1"), None] {
		let selected = select_exit_container("web", "db", "proj-db-1", &names, |name| async move {
			Ok(stopped == Some(name.as_str()))
		})
		.await
		.unwrap();
		assert_eq!(selected, stopped.unwrap_or("proj-web-1"));
	}
}
