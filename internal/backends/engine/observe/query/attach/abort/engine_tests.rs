#![cfg(unix)]

use super::super::super::{AttachOptions, Engine};
use crate::engine::fake_podman::{self, FakeReply};

fn file() -> crate::compose::types::ComposeFile {
	crate::parse_str("services:\n  web:\n    image: img\n    scale: 2\n").unwrap()
}

fn engine(fake: &fake_podman::FakePodman) -> Engine {
	Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir())
}

fn fake_wait(reply: u8) -> fake_podman::FakePodman {
	fake_podman::start_replying(move |method, target| {
		if target.contains("proj-web-1/logs") {
			// A broken stream cannot be the exit trigger; replica 2 wins.
			FakeReply::Body(500, r#"{"message":"logs unavailable"}"#.into())
		} else if target.contains("/logs") {
			FakeReply::ChunkedEnd(vec![])
		} else if target.contains("/wait?") {
			match reply {
				0 => FakeReply::Body(500, r#"{"message":"wait unavailable"}"#.into()),
				1 => FakeReply::Body(200, "invalid".into()),
				2 => FakeReply::ClosedWithoutResponse,
				3 => FakeReply::Body(200, "0".into()),
				_ if target.contains("proj-web-2/") => FakeReply::Body(200, "7".into()),
				_ => FakeReply::Body(200, "137".into()),
			}
		} else if target.contains("/containers/json") {
			FakeReply::Body(200, r#"[{"Names":["proj-web-1"],"State":"running","Labels":{"podup.project":"proj","podup.service":"web"}},{"Names":["proj-web-2"],"State":"exited","Labels":{"podup.project":"proj","podup.service":"web"}}]"#.into())
		} else if method == "POST" {
			FakeReply::Body(204, String::new())
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.into())
		}
	})
}

#[tokio::test]
async fn f8_engine_wait_failures_abort_and_exit_code_from() {
	for reply in 0..3 {
		for named in [false, true] {
			let fake = fake_wait(reply);
			let options = AttachOptions::new()
				.with_abort_on_container_exit(true)
				.with_exit_code_from(named.then(|| "web".into()));
			let error = engine(&fake)
				.attach_logs_with(&file(), &options)
				.await
				.unwrap_err()
				.to_string();
			assert!(error.contains("service web"), "{error}");
			assert!(error.contains("proj-web-2"), "{error}");
			let requests = fake.requests.lock().unwrap();
			assert!(
				requests.iter().any(|r| r.contains("proj-web-1/stop?")),
				"{requests:?}"
			);
		}
	}
}

#[tokio::test]
async fn f8_engine_wait_zero_remains_success() {
	for named in [false, true] {
		let fake = fake_wait(3);
		let options = AttachOptions::new()
			.with_abort_on_container_exit(true)
			.with_exit_code_from(named.then(|| "web".into()));
		let summary = engine(&fake)
			.attach_logs_with(&file(), &options)
			.await
			.unwrap();
		assert_eq!(summary.exit_code, Some(0));
	}
}

#[tokio::test]
async fn f8_engine_exit_code_from_uses_second_replica() {
	let fake = fake_wait(4);
	let options = AttachOptions::new().with_exit_code_from(Some("web".into()));
	let summary = engine(&fake)
		.attach_logs_with(&file(), &options)
		.await
		.unwrap();
	assert_eq!(summary.exit_code, Some(7));
	let requests = fake.requests.lock().unwrap();
	assert!(requests.iter().any(|r| r.contains("proj-web-2/wait?")));
	assert!(!requests.iter().any(|r| r.contains("proj-web-1/wait?")));
}

/// The container that ended the run exited 0 and the rest could not be stopped:
/// the project is still up, so `up --abort-on-container-exit` must not say 0.
#[tokio::test]
async fn f3_engine_a_failed_stop_after_a_zero_exit_is_an_error() {
	let fake = fake_podman::start_replying(|method, target| {
		if target.contains("proj-web-1/logs") {
			FakeReply::Body(500, r#"{"message":"logs unavailable"}"#.into())
		} else if target.contains("/logs") {
			FakeReply::ChunkedEnd(vec![])
		} else if target.contains("/wait?") {
			FakeReply::Body(200, "0".into())
		} else if target.contains("/containers/json") {
			FakeReply::Body(200, r#"[{"Names":["proj-web-1"],"State":"running","Labels":{"podup.project":"proj","podup.service":"web"}},{"Names":["proj-web-2"],"State":"exited","Labels":{"podup.project":"proj","podup.service":"web"}}]"#.into())
		} else if method == "POST" && target.contains("/stop?") {
			FakeReply::Body(500, r#"{"message":"stop refused"}"#.into())
		} else if method == "POST" {
			FakeReply::Body(204, String::new())
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.into())
		}
	});
	let options = AttachOptions::new().with_abort_on_container_exit(true);
	let error = engine(&fake)
		.attach_logs_with(&file(), &options)
		.await
		.expect_err("the stop failed, so the project is not down");
	assert!(error.to_string().contains("stop refused"), "{error}");
}

/// Two replicas of one service: the first to end exits cleanly, the second
/// fails. Every stream ends at once, so which ends first is not up to the test.
fn failing_second_replica() -> fake_podman::FakePodman {
	fake_podman::start_replying(|method, target| {
		if target.contains("/logs") {
			FakeReply::ChunkedEnd(vec![])
		} else if target.contains("proj-web-2/wait?") {
			FakeReply::Body(200, "7".into())
		} else if target.contains("/wait?") {
			FakeReply::Body(200, "0".into())
		} else if target.contains("/containers/json") {
			FakeReply::Body(200, "[]".into())
		} else if method == "POST" {
			FakeReply::Body(204, String::new())
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.into())
		}
	})
}

#[tokio::test]
async fn abort_on_container_failure_skips_clean_exits_and_stops_on_the_failure() {
	let fake = failing_second_replica();
	let options = AttachOptions::new().with_abort_on_container_failure(true);
	let summary = engine(&fake)
		.attach_logs_with(&file(), &options)
		.await
		.unwrap();
	assert_eq!(summary.outcome, super::super::AttachOutcome::Aborted);
	assert_eq!(summary.exit_code, Some(7));
	assert_eq!(summary.service.as_deref(), Some("web"));
}

#[tokio::test]
async fn abort_on_container_failure_lets_an_all_clean_run_finish() {
	let fake = fake_podman::start_replying(|method, target| {
		if target.contains("/logs") {
			FakeReply::ChunkedEnd(vec![])
		} else if target.contains("/wait?") {
			FakeReply::Body(200, "0".into())
		} else if method == "POST" {
			FakeReply::Body(204, String::new())
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.into())
		}
	});
	let options = AttachOptions::new().with_abort_on_container_failure(true);
	let summary = engine(&fake)
		.attach_logs_with(&file(), &options)
		.await
		.unwrap();
	assert_eq!(summary.outcome, super::super::AttachOutcome::StreamsEnded);
	assert_eq!(summary.exit_code, None);
}
