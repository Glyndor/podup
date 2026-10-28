//! The wait that follows a kill is bounded.
//!
//! The Docker compat `/kill` handler blocks on the container when the signal
//! is SIGKILL (or the existence probe 0), and answers once the container has
//! exited or stopped. libpod's handler answers immediately, so podup
//! compensates with a follow-up `wait?condition=stopped` for those signals.
//!
//! That wait used to have no deadline of its own: a daemon that took the
//! request and never answered it left the command waiting forever, with
//! nothing to interrupt it. The kill itself had already been delivered and
//! acknowledged, so the wait is an observation, and an observation that never
//! arrives must not pin the CLI.

use crate::engine::fake_podman::{self, FakePodman, FakeReply};
use crate::engine::Engine;

/// The project's containers: one replica of `web`.
const LIVE: &str =
	r#"[{"Names":["/proj-web-1"],"State":"running","Labels":{"podup.service":"web"}}]"#;

/// A fake whose `wait?condition=stopped` is never answered: the daemon takes
/// the request and stays silent, which is the shape that used to hang the
/// command.
fn silent_daemon() -> FakePodman {
	fake_podman::start_replying(|method, target| {
		if method == "GET" && target.contains("/containers/json") {
			return FakeReply::Body(200, LIVE.to_string());
		}
		if target.contains("/wait") {
			return FakeReply::Silent;
		}
		FakeReply::Body(200, String::new())
	})
}

fn file() -> crate::compose::types::ComposeFile {
	crate::parse_str("services:\n  web:\n    image: alpine\n").expect("the fixture parses")
}

fn engine(fake: &FakePodman) -> Engine {
	Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir())
}

/// The wait gives up on its own instead of waiting for the daemon forever.
///
/// The bound is passed in so the test does not have to wait out the
/// production one; what is under test is that the wait ends on its own.
#[tokio::test]
async fn the_wait_after_a_kill_gives_up_on_its_own() {
	let fake = silent_daemon();
	let engine = engine(&fake);

	let outcome = tokio::time::timeout(
		std::time::Duration::from_secs(20),
		engine.wait_after_kill("proj-web-1", std::time::Duration::from_millis(200)),
	)
	.await;

	assert!(
		outcome.is_ok(),
		"the wait must end on its own rather than wait for the daemon forever"
	);
}

/// The command as a whole returns even when the daemon never answers the
/// wait.
///
/// This is the shape a user hits: the signal was delivered, and the command
/// still has to come back.
#[tokio::test]
async fn the_kill_returns_when_the_daemon_never_answers_the_wait() {
	let fake = silent_daemon();
	let engine = engine(&fake);

	let outcome = tokio::time::timeout(
		std::time::Duration::from_secs(30),
		engine.kill(&file(), &[], "SIGKILL"),
	)
	.await;

	assert!(
		outcome.is_ok(),
		"the kill must return even when the daemon never answers the wait"
	);
	assert!(
		outcome.expect("the kill returned").is_ok(),
		"the kill itself succeeded, so the wait must not turn it into a failure"
	);
}
