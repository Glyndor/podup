#![cfg(unix)]

use crate::engine::fake_podman::{self, FakeReply};
use crate::engine::{Engine, PullOptions};
use crate::ui::progress::capture::Capture;

fn stale_image_engine(kind: u8) -> (fake_podman::FakePodman, Engine) {
	let fake = fake_podman::start_replying(move |method, target| {
		if method == "POST" && target.contains("/images/pull?") {
			match kind {
				0 => FakeReply::ChunkedTruncated(vec![
					"{\"stream\":\"Copying blob sha256:aaa\"}\n".into(),
				]),
				1 => FakeReply::ChunkedCutMidPayload(
					"{\"stream\":\"Copying blob sha256:aaa\"}\n".into(),
				),
				2 => FakeReply::Body(200, "not json\n".into()),
				_ => FakeReply::Body(200, "{\"stream\":".into()),
			}
		} else if method == "GET" && target.contains("/images/") {
			FakeReply::Body(200, "{}".into())
		} else {
			FakeReply::Body(404, "{}".into())
		}
	});
	let engine = Engine::new(fake.client(), "proj".into());
	(fake, engine)
}

#[tokio::test]
async fn truncated_or_malformed_pull_cannot_succeed_with_a_stale_image() {
	for kind in 0..4 {
		let (_fake, engine) = stale_image_engine(kind);
		let file =
			crate::parse_str("services:\n  app:\n    image: stale:v1\n    pull_policy: always\n")
				.unwrap();
		let capture = Capture::start();
		assert!(engine.pull(&file).await.is_err());
		let verbs = capture.verbs();
		assert!(verbs.iter().any(|(_, _, verb)| verb == "Failed"));
		assert!(!verbs.iter().any(|(_, _, verb)| verb == "Pulled"));
		assert!(capture.every_board_ended());
	}
}

#[tokio::test]
async fn explicit_ignore_failures_still_accepts_a_truncated_pull() {
	let (_fake, engine) = stale_image_engine(0);
	let file = crate::parse_str("services:\n  app:\n    image: stale:v1\n").unwrap();
	engine
		.pull_services_with_options(
			&file,
			&[],
			PullOptions {
				ignore_failures: true,
				..Default::default()
			},
		)
		.await
		.expect("the explicit ignore option still wins");
}
