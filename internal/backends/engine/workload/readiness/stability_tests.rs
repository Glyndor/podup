use super::*;

fn inspect(status: &str, code: i64, restarts: i64, policy: &str) -> String {
	format!(
		r#"{{"State":{{"Status":"{status}","ExitCode":{code}}},"RestartCount":{restarts},"HostConfig":{{"RestartPolicy":{{"Name":"{policy}"}}}},"Config":{{}}}}"#
	)
}

fn parsed(json: &str) -> ContainerInspect {
	serde_json::from_str(json).expect("fixture parses")
}

// --- the reading ---------------------------------------------------------

#[test]
fn a_running_container_says_whether_a_policy_can_start_it_again() {
	for (policy, can) in [
		("no", false),
		("", false),
		("always", true),
		("unless-stopped", true),
		("on-failure", true),
	] {
		assert_eq!(
			read(&parsed(&inspect("running", 0, 2, policy))),
			Reading::Running {
				restarts: 2,
				can_restart: can
			},
			"{policy:?}"
		);
	}
}

#[test]
fn a_failure_with_no_restart_policy_is_an_exit() {
	assert_eq!(
		read(&parsed(&inspect("exited", 3, 0, "no"))),
		Reading::Exited(3)
	);
}

#[test]
fn a_clean_one_shot_is_fine_and_a_clean_exit_under_always_is_a_loop() {
	assert_eq!(read(&parsed(&inspect("exited", 0, 0, "no"))), Reading::Fine);
	assert_eq!(
		read(&parsed(&inspect("exited", 0, 0, "on-failure"))),
		Reading::Fine
	);
	for policy in ["always", "unless-stopped"] {
		assert_eq!(
			read(&parsed(&inspect("stopped", 0, 4, policy))),
			Reading::Restarting {
				code: Some(0),
				restarts: 4
			},
			"{policy}"
		);
	}
}

/// Podman reports `stopped` between two runs of a policy that restarts a
/// failing container (measured on 6.2.0: `stopped`, exit 3, RestartCount 3).
#[test]
fn a_stopped_container_a_policy_keeps_starting_is_a_crash_loop() {
	assert_eq!(
		read(&parsed(&inspect("stopped", 3, 3, "on-failure"))),
		Reading::Restarting {
			code: Some(3),
			restarts: 3
		}
	);
	assert_eq!(
		read(&parsed(&inspect("stopped", 3, 0, "no"))),
		Reading::Exited(3),
		"no restarts and no policy: it simply ended"
	);
}

#[test]
fn restarting_and_odd_states() {
	assert!(matches!(
		read(&parsed(&inspect("restarting", 1, 1, "always"))),
		Reading::Restarting { .. }
	));
	for status in ["created", "paused", "configured", "removing", ""] {
		assert_eq!(read(&parsed(&inspect(status, 0, 0, "no"))), Reading::Fine);
	}
	// Nothing at all in the answer is no opinion.
	assert_eq!(read(&parsed("{}")), Reading::Fine);
	assert_eq!(
		read(&parsed(r#"{"State":null,"HostConfig":null}"#)),
		Reading::Fine
	);
}

/// The wait against a fake socket; the fake listens on a unix socket.
#[cfg(unix)]
mod waits {
	use std::sync::atomic::{AtomicUsize, Ordering};
	use std::sync::Arc;

	use super::*;
	use crate::engine::fake_podman;
	use crate::error::ComposeError;

	/// A fake that answers the container inspect from a list, one entry per read,
	/// the last one repeated.
	fn fake_sequence(answers: Vec<String>) -> (fake_podman::FakePodman, Arc<AtomicUsize>) {
		let reads = Arc::new(AtomicUsize::new(0));
		let counter = Arc::clone(&reads);
		let fake = fake_podman::start(move |method, target| {
			if method == "GET" && target.contains("/containers/") && target.ends_with("/json") {
				let n = counter.fetch_add(1, Ordering::SeqCst);
				let at = n.min(answers.len() - 1);
				return (200, answers[at].clone());
			}
			(404, r#"{"message":"not found"}"#.to_string())
		});
		(fake, reads)
	}

	fn engine(fake: &fake_podman::FakePodman) -> Engine {
		Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir())
	}

	#[tokio::test(start_paused = true)]
	async fn a_service_that_keeps_running_passes_after_the_window() {
		let (fake, reads) = fake_sequence(vec![inspect("running", 0, 0, "unless-stopped")]);
		engine(&fake)
			.wait_stable("proj-web-1", None)
			.await
			.expect("a stable service passes");
		assert!(
			reads.load(Ordering::SeqCst) > 5,
			"the window reads the state more than once"
		);
	}

	/// The case behind this: `sh -c 'sleep 1; exit 3'` under `restart:
	/// unless-stopped` is running when the wait first looks, and is started again
	/// a second later.
	#[tokio::test(start_paused = true)]
	async fn a_restart_inside_the_window_fails_the_wait() {
		let (fake, _) = fake_sequence(vec![
			inspect("running", 0, 0, "unless-stopped"),
			inspect("running", 0, 0, "unless-stopped"),
			inspect("running", 0, 1, "unless-stopped"),
		]);
		let error = engine(&fake)
			.wait_stable("proj-web-1", None)
			.await
			.expect_err("a crash loop is not running");
		assert!(
			matches!(
				error,
				ComposeError::WaitServiceRestarting { restarts: 1, .. }
			),
			"{error}"
		);
		assert!(error.to_string().contains("keeps restarting"), "{error}");
	}

	#[tokio::test(start_paused = true)]
	async fn a_container_caught_between_two_runs_fails_with_its_exit_code() {
		let (fake, _) = fake_sequence(vec![
			inspect("running", 0, 0, "always"),
			inspect("stopped", 3, 1, "always"),
		]);
		let error = engine(&fake)
			.wait_stable("proj-web-1", None)
			.await
			.expect_err("between two runs is not running");
		assert!(
			matches!(
				error,
				ComposeError::WaitServiceRestarting { code: Some(3), .. }
			),
			"{error}"
		);
	}

	#[tokio::test(start_paused = true)]
	async fn a_failure_with_no_policy_fails_at_the_first_look() {
		let (fake, reads) = fake_sequence(vec![inspect("exited", 3, 0, "no")]);
		let error = engine(&fake)
			.wait_stable("proj-web-1", None)
			.await
			.expect_err("a dead service is not ready");
		assert!(
			matches!(error, ComposeError::WaitServiceExited { code: 3, .. }),
			"{error}"
		);
		assert_eq!(reads.load(Ordering::SeqCst), 1);
	}

	/// A one-shot that finished well, and a service whose policy never restarts it,
	/// are not held for a window: nothing could change.
	#[tokio::test(start_paused = true)]
	async fn a_clean_one_shot_and_a_service_without_a_policy_pass_at_the_first_look() {
		for json in [
			inspect("exited", 0, 0, "no"),
			inspect("running", 0, 0, "no"),
		] {
			let (fake, reads) = fake_sequence(vec![json]);
			engine(&fake)
				.wait_stable("proj-web-1", None)
				.await
				.expect("fine");
			assert_eq!(reads.load(Ordering::SeqCst), 1);
		}
	}

	#[tokio::test(start_paused = true)]
	async fn a_container_with_a_healthcheck_is_left_to_its_health() {
		let json = r#"{"State":{"Status":"running"},"RestartCount":9,"HostConfig":{"RestartPolicy":{"Name":"always"}},"Config":{"Healthcheck":{"Test":["CMD","true"]}}}"#;
		let (fake, reads) = fake_sequence(vec![json.to_string()]);
		engine(&fake)
			.wait_stable("proj-web-1", None)
			.await
			.expect("health is the answer");
		assert_eq!(reads.load(Ordering::SeqCst), 1);
	}

	/// The window never outlasts what the caller has left: a `--wait-timeout` of
	/// one second must not be held for three.
	#[tokio::test(start_paused = true)]
	async fn the_window_is_bounded_by_the_time_left() {
		let (fake, _) = fake_sequence(vec![inspect("running", 0, 0, "always")]);
		let started = tokio::time::Instant::now();
		engine(&fake)
			.wait_stable("proj-web-1", Some(Duration::from_secs(1)))
			.await
			.expect("stable");
		assert!(started.elapsed() <= Duration::from_millis(1200));
	}

	#[tokio::test(start_paused = true)]
	async fn an_inspect_that_fails_inside_the_window_is_not_a_verdict() {
		let reads = Arc::new(AtomicUsize::new(0));
		let counter = Arc::clone(&reads);
		let fake = fake_podman::start(move |_, target| {
			if target.ends_with("/json") {
				if counter.fetch_add(1, Ordering::SeqCst) == 0 {
					return (200, inspect("running", 0, 0, "always"));
				}
				return (500, r#"{"message":"busy"}"#.to_string());
			}
			(404, "{}".to_string())
		});
		engine(&fake)
			.wait_stable("proj-web-1", None)
			.await
			.expect("a read that failed says nothing");
	}
}
