use super::*;

#[tokio::test]
async fn join_bounded_preserves_input_order() {
	// Futures finish out of order (later index resolves first) but the
	// collected results are returned in input order.
	let futs = (0..5usize).map(|i| async move {
		// Smaller i yields later via more yields, so completion order != input.
		for _ in 0..(5 - i) {
			tokio::task::yield_now().await;
		}
		if i == 2 {
			Err(ComposeError::ServiceNotFound(format!("svc{i}")))
		} else {
			Ok(())
		}
	});
	let results = join_bounded(futs).await;
	assert_eq!(results.len(), 5);
	// Only index 2 is an error, proving order is preserved.
	for (i, r) in results.iter().enumerate() {
		assert_eq!(r.is_err(), i == 2, "index {i}");
	}
}

#[test]
fn first_error_returns_first_in_order() {
	let results = vec![
		Ok(()),
		Err(ComposeError::ServiceNotFound("a".into())),
		Err(ComposeError::ServiceNotFound("b".into())),
	];
	let err = first_error(results).unwrap();
	assert!(matches!(err, ComposeError::ServiceNotFound(n) if n == "a"));
}

#[test]
fn first_error_none_when_all_ok() {
	assert!(first_error(vec![Ok(()), Ok(())]).is_none());
}
