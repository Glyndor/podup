//! The `--rm` cleanup of a one-off container is part of what the command was
//! asked to do. A container left behind keeps its writable layer, and with it
//! whatever the command wrote, so a cleanup that fails has to be visible.

use crate::engine::fake_podman::{self, FakeReply};
use crate::engine::Engine;

fn compose() -> crate::compose::types::ComposeFile {
	crate::parse_str("services:\n  app:\n    image: img\n").unwrap()
}

/// One stdout frame in the multiplexed shape Podman sends for a non-TTY
/// container.
fn stdout_frame(payload: &str) -> String {
	let mut frame = vec![0x01u8, 0, 0, 0];
	frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
	frame.extend_from_slice(payload.as_bytes());
	String::from_utf8(frame).expect("the frame is ascii")
}

/// A fake whose container deletion answers with `delete_status` and whose
/// command exits with `code`.
fn fake(delete_status: u16, code: i64) -> fake_podman::FakePodman {
	fake_podman::start_replying(move |method, target| {
		if method == "DELETE" {
			FakeReply::Body(delete_status, r#"{"message":"cannot remove"}"#.to_string())
		} else if target.contains("/logs") {
			FakeReply::ChunkedEnd(vec![stdout_frame("done\n")])
		} else if target.contains("/wait") {
			FakeReply::Body(200, code.to_string())
		} else if target.contains("/containers/json") {
			FakeReply::Body(
				200,
				r#"[{"Names":["/proj-app-run-1"],"State":"exited"}]"#.to_string(),
			)
		} else if method == "POST" {
			FakeReply::Body(200, r#"{"Id":"cafe"}"#.to_string())
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.to_string())
		}
	})
}

fn options() -> crate::engine::RunOptions {
	crate::engine::RunOptions {
		cmd: vec![],
		rm: true,
		detach: false,
		env_overrides: vec![],
		name_override: Some("proj-app-run-1".to_string()),
		service_ports: false,
	}
}

fn engine(fake: &fake_podman::FakePodman) -> Engine {
	Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir())
}

/// Whether the `--rm` cleanup was attempted.
///
/// Cleanup addresses the ID returned by this run's create response.
fn delete_attempted(fake: &fake_podman::FakePodman) -> bool {
	fake.requests
		.lock()
		.unwrap()
		.iter()
		.any(|r| r.starts_with("DELETE ") && r.ends_with("/containers/cafe?force=true"))
}

/// A command that ran fine but whose container could not be removed is not a
/// successful `run`: the removal was asked for explicitly, and the container
/// is still there with everything the command wrote in it.
#[tokio::test]
async fn a_failed_removal_fails_a_run_whose_command_succeeded() {
	let fake = fake(500, 0);
	let e = engine(&fake);

	let err = e
		.run(&compose(), "app", options())
		.await
		.expect_err("the container could not be removed, so the run did not do what it was asked");
	assert!(
		matches!(err, crate::error::ComposeError::Podman(_)),
		"expected the removal failure to surface, got {err:?}"
	);
	assert!(
		delete_attempted(&fake),
		"the removal was never attempted, so the test proves nothing"
	);
}

/// A container that is already gone is the idempotent success `--rm` implies:
/// nothing is left behind, so nothing failed.
#[tokio::test]
async fn an_already_removed_container_is_not_a_failure() {
	let fake = fake(404, 0);
	let e = engine(&fake);

	e.run(&compose(), "app", options())
		.await
		.expect("a container that is already gone is the outcome --rm asked for");
}

/// When the command itself failed, its own error is the one reported: the
/// exit code is what the caller needs. The incomplete cleanup is reported
/// next to it, not in place of it.
#[tokio::test]
async fn a_failed_command_keeps_its_own_error_when_the_removal_also_fails() {
	let fake = fake(500, 7);
	let e = engine(&fake);

	let err = e
		.run(&compose(), "app", options())
		.await
		.expect_err("the command exited with 7");
	assert!(
		matches!(err, crate::error::ComposeError::RunExited(7)),
		"expected the command's own exit code, got {err:?}"
	);
	assert!(
		delete_attempted(&fake),
		"the removal was never attempted, so the test proves nothing"
	);
}

/// Without `--rm` there is no removal to fail, so the command's own result is
/// the only thing that decides the exit status.
#[tokio::test]
async fn without_rm_the_command_result_is_the_only_one() {
	let fake = fake(500, 0);
	let e = engine(&fake);
	let mut opts = options();
	opts.rm = false;

	e.run(&compose(), "app", opts)
		.await
		.expect("nothing was asked to be removed, so nothing failed");
	assert!(
		!delete_attempted(&fake),
		"a run without --rm must not remove anything"
	);
}

/// Another project wins the name after run's initial existence check.
#[tokio::test]
async fn rm_does_not_delete_a_container_rejected_by_the_ownership_check() {
	use std::sync::atomic::{AtomicUsize, Ordering};

	let inspections = std::sync::Arc::new(AtomicUsize::new(0));
	let count = inspections.clone();
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.ends_with("/containers/proj-app-run-1/json") {
			if count.fetch_add(1, Ordering::SeqCst) == 0 {
				return (404, r#"{"message":"not found"}"#.into());
			}
			return (
				200,
				r#"{"Id":"foreign-id","Config":{"Labels":{"podup.project":"other"}}}"#.into(),
			);
		}
		if method == "POST" {
			(200, r#"{"Id":"own-id"}"#.into())
		} else {
			(404, r#"{"message":"not found"}"#.into())
		}
	});
	let err = engine(&fake)
		.run(&compose(), "app", options())
		.await
		.unwrap_err();
	assert!(matches!(err, crate::error::ComposeError::Unsupported(_)));
	assert!(inspections.load(Ordering::SeqCst) >= 2);
	let seen = fake.requests.lock().unwrap();
	assert!(!seen.iter().any(|r| r.starts_with("DELETE ")));
	assert!(!seen.iter().any(|r| r.ends_with("/containers/create")));
}

#[tokio::test]
async fn rm_does_not_delete_anything_when_creation_loses_a_name_race() {
	let fake = fake_podman::start(|method, target| {
		if method == "POST" && target.ends_with("/containers/create") {
			(409, r#"{"message":"name already in use"}"#.into())
		} else if method == "POST" {
			(200, r#"{"Id":"unused"}"#.into())
		} else {
			(404, r#"{"message":"not found"}"#.into())
		}
	});
	let err = engine(&fake)
		.run(&compose(), "app", options())
		.await
		.unwrap_err();
	assert!(matches!(err, crate::error::ComposeError::Podman(ref e) if e.is_status(409)));
	let seen = fake.requests.lock().unwrap();
	assert!(seen.iter().any(|r| r.ends_with("/containers/create")));
	assert!(!seen.iter().any(|r| r.starts_with("DELETE ")));
}

#[tokio::test]
async fn a_failed_start_removes_only_the_id_returned_by_this_creation() {
	let fake = fake_podman::start(|method, target| {
		if method == "POST" && target.ends_with("/start") {
			(500, r#"{"message":"start refused"}"#.into())
		} else if method == "POST" {
			(200, r#"{"Id":"own-id"}"#.into())
		} else if method == "DELETE" {
			(204, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.into())
		}
	});
	let err = engine(&fake)
		.run(&compose(), "app", options())
		.await
		.unwrap_err();
	assert!(matches!(err, crate::error::ComposeError::Podman(ref e) if e.is_status(500)));
	let deletes: Vec<_> = fake
		.requests
		.lock()
		.unwrap()
		.iter()
		.filter(|r| r.starts_with("DELETE "))
		.cloned()
		.collect();
	assert_eq!(
		deletes,
		vec![format!(
			"DELETE {}/containers/own-id?force=true",
			crate::libpod::API_PREFIX
		)]
	);
}

#[tokio::test]
async fn successful_run_cleanup_uses_its_creation_id_instead_of_its_name() {
	let fake = fake(204, 0);
	engine(&fake)
		.run(&compose(), "app", options())
		.await
		.unwrap();
	let deletes: Vec<_> = fake
		.requests
		.lock()
		.unwrap()
		.iter()
		.filter(|r| r.starts_with("DELETE "))
		.cloned()
		.collect();
	assert_eq!(
		deletes,
		vec![format!(
			"DELETE {}/containers/cafe?force=true",
			crate::libpod::API_PREFIX
		)]
	);
}
