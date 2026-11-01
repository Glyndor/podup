use super::*;
use std::time::Duration;

async fn completed(name: &'static str, delay: u64, code: i64) -> (String, Result<i64>) {
	tokio::time::sleep(Duration::from_secs(delay)).await;
	(name.into(), Ok(code))
}

#[tokio::test(start_paused = true)]
async fn f8_last_completion_zero_wins_in_both_argument_orders() {
	for reversed in [false, true] {
		let mut waits = vec![completed("a", 1, 7), completed("b", 2, 0)];
		if reversed {
			waits.reverse();
		}
		let mut rows = Vec::new();
		wait_for_containers(waits, |name, code| rows.push((name.to_string(), code)))
			.await
			.unwrap();
		assert_eq!(rows, [("a".into(), 7), ("b".into(), 0)]);
	}
}

#[tokio::test(start_paused = true)]
async fn f8_last_completion_failure_wins_in_both_argument_orders() {
	for reversed in [false, true] {
		let mut waits = vec![completed("a", 2, 7), completed("b", 1, 9)];
		if reversed {
			waits.reverse();
		}
		let mut rows = Vec::new();
		let error = wait_for_containers(waits, |name, code| rows.push((name.to_string(), code)))
			.await
			.unwrap_err();
		assert!(matches!(error, ComposeError::RunExited(7)), "{error}");
		assert_eq!(rows, [("b".into(), 9), ("a".into(), 7)]);
	}
}

#[tokio::test(start_paused = true)]
async fn f8_waits_start_concurrently() {
	let (send, receive) = tokio::sync::oneshot::channel();
	let waits: Vec<futures_util::future::LocalBoxFuture<'_, (String, Result<i64>)>> = vec![
		Box::pin(async {
			receive.await.unwrap();
			("a".into(), Ok(0))
		}),
		Box::pin(async {
			send.send(()).unwrap();
			("b".into(), Ok(0))
		}),
	];
	tokio::time::timeout(
		Duration::from_secs(1),
		wait_for_containers(waits, |_, _| {}),
	)
	.await
	.expect("both waits must start before either finishes")
	.unwrap();
}

#[tokio::test(start_paused = true)]
async fn f8_wait_error_survives_later_zero_and_drains_waits() {
	let waits = [true, false].map(|fail| async move {
		if fail {
			(
				"a".into(),
				Err(ComposeError::Io(std::io::Error::other("wait failed"))),
			)
		} else {
			completed("b", 1, 0).await
		}
	});
	let mut rows = Vec::new();
	let error = wait_for_containers(waits, |name, code| rows.push((name.to_string(), code)))
		.await
		.unwrap_err();
	assert!(error.to_string().contains("wait failed"));
	assert_eq!(rows, [("b".into(), 0)]);
}
