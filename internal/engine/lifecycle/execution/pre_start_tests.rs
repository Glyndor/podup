use super::{execute_steps, Action};
use crate::compose::types::Service;
use crate::error::ComposeError;
use std::cell::RefCell;

#[tokio::test]
async fn pre_start_runs_and_removes_each_step_before_the_next() {
	let events = RefCell::new(Vec::new());
	execute_steps(&[Service::default(), Service::default()], |i, action| {
		events
			.borrow_mut()
			.push((i, matches!(action, Action::Remove)));
		std::future::ready(Ok(0))
	})
	.await
	.unwrap();
	assert_eq!(
		*events.borrow(),
		[(0, false), (0, true), (1, false), (1, true)]
	);
}

#[tokio::test]
async fn pre_start_failure_removes_container_and_stops_later_steps() {
	let events = RefCell::new(Vec::new());
	let error = execute_steps(&[Service::default(), Service::default()], |i, action| {
		events
			.borrow_mut()
			.push((i, matches!(action, Action::Remove)));
		std::future::ready(Ok(if matches!(action, Action::Run) { 7 } else { 0 }))
	})
	.await
	.unwrap_err()
	.to_string();
	assert!(error.contains("pre_start step 1"), "{error}");
	assert!(error.contains('7'), "{error}");
	assert_eq!(*events.borrow(), [(0, false), (0, true)]);
}

#[tokio::test]
async fn pre_start_start_or_wait_error_still_removes_container() {
	let events = RefCell::new(Vec::new());
	let error = execute_steps(&[Service::default()], |i, action| {
		events.borrow_mut().push(i);
		std::future::ready(match action {
			Action::Run => Err(ComposeError::Unsupported("start or wait failed".into())),
			Action::Remove => Ok(0),
		})
	})
	.await
	.unwrap_err()
	.to_string();
	assert!(error.contains("pre_start step 1"), "{error}");
	assert!(error.contains("start or wait failed"), "{error}");
	assert_eq!(*events.borrow(), [0, 0]);
}

#[tokio::test]
async fn pre_start_cleanup_error_prevents_service_creation() {
	let error = execute_steps(&[Service::default()], |_, action| {
		std::future::ready(match action {
			Action::Run => Ok(0),
			Action::Remove => Err(ComposeError::Unsupported("remove failed".into())),
		})
	})
	.await
	.unwrap_err()
	.to_string();
	assert!(error.contains("pre_start step 1"), "{error}");
	assert!(error.contains("remove failed"), "{error}");
}
