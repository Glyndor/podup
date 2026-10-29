//! The deadlines the per-service lifecycle units carry.
//!
//! Two of them are about how long podup is willing to wait for the daemon.
//!
//! The wait that follows a kill: the Docker compat `/kill` handler blocks on
//! the container when the signal is SIGKILL (or the existence probe 0), and
//! answers once the container has exited or stopped. libpod's handler answers
//! immediately, so podup compensates with a follow-up
//! `wait?condition=stopped` for those signals. That wait used to have no
//! deadline of its own: a daemon that took the request and never answered it
//! left the command waiting forever, with nothing to interrupt it. The kill
//! itself had already been delivered and acknowledged, so the wait is an
//! observation, and an observation that never arrives must not pin the CLI.
//!
//! The deadline a restart carries: the daemon is told how long it may take to
//! stop the container, so the client has to be willing to wait at least that
//! long. The default read timeout cut a restart that was still inside the
//! grace the user or the service asked for.

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

/// A fake whose restart request is never answered either. The clock is frozen
/// when the request arrives, so the client's own deadline is what ends the
/// wait, and the frozen clock reaches it without the test waiting for it.
fn frozen_daemon() -> FakePodman {
	fake_podman::start_replying(|method, target| {
		if method == "GET" && target.contains("/containers/json") {
			return FakeReply::Body(200, LIVE.to_string());
		}
		if target.contains("/restart") {
			// The listing is already done, so nothing else is waiting on the
			// clock: freezing it here leaves the client's deadline as the only
			// timer, and the runtime advances to it.
			tokio::time::pause();
			return FakeReply::Silent;
		}
		FakeReply::Body(200, String::new())
	})
}

fn file() -> crate::compose::types::ComposeFile {
	crate::parse_str("services:\n  web:\n    image: alpine\n").expect("the fixture parses")
}

/// A service whose shutdown grace is longer than the client's default read
/// timeout.
fn file_with_a_long_grace() -> crate::compose::types::ComposeFile {
	crate::parse_str("services:\n  web:\n    image: alpine\n    stop_grace_period: 180s\n")
		.expect("the fixture parses")
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

/// A restart waits for the grace it was given.
///
/// The service asks for three minutes of shutdown grace, and the daemon is
/// told so. The client has to wait at least that long too: the default read
/// timeout would report a failure while the restart was still inside the
/// grace the service asked for.
#[tokio::test]
async fn a_restart_waits_for_the_grace_it_was_given() {
	let fake = frozen_daemon();
	let engine = engine(&fake);

	let error = engine
		.restart_with_options(&file_with_a_long_grace(), &["web".to_string()], false)
		.await
		.expect_err("the daemon never answers, so the restart ends in a timeout");

	let message = error.to_string();
	assert!(
		message.contains("210s"),
		"the client must wait for the grace it was given, not the default read \
		 timeout, got: {message}"
	);
}

/// A restart is a stop plus a start, so a short grace must not shrink its
/// deadline below the client's default read timeout (120s).
#[tokio::test]
async fn a_restart_with_a_short_grace_keeps_the_default_deadline() {
	let fake = frozen_daemon();
	let engine = engine(&fake);

	let error = engine
		.restart_with_options(&file(), &["web".to_string()], false)
		.await
		.expect_err("the daemon never answers, so the restart ends in a timeout");

	let message = error.to_string();
	assert!(
		message.contains("120s"),
		"a short grace cut the restart below the default deadline, got: {message}"
	);
}
