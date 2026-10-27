use super::{
	expand_exec_env, is_exec_teardown_noise, map_exec_start_err, map_not_running, ExecOptions,
	EXEC_START_TIMEOUT,
};

#[test]
fn expand_exec_env_passes_through_key_value() {
	let out = expand_exec_env(&["FOO=bar".to_string(), "BAZ=qux".to_string()]);
	assert_eq!(out, vec!["FOO=bar".to_string(), "BAZ=qux".to_string()]);
}

#[test]
fn expand_exec_env_resolves_bare_key_from_host() {
	// A bare `KEY` takes its value from podup's own environment; an unset bare
	// key is dropped (libpod rejects a value-less env entry).
	std::env::set_var("PODUP_TEST_EXEC_ENV", "from-host");
	let out = expand_exec_env(&[
		"PODUP_TEST_EXEC_ENV".to_string(),
		"PODUP_TEST_EXEC_UNSET_ENV".to_string(),
	]);
	std::env::remove_var("PODUP_TEST_EXEC_ENV");
	assert_eq!(out, vec!["PODUP_TEST_EXEC_ENV=from-host".to_string()]);
}

#[test]
fn teardown_noise_matches_only_connection_reset_frame() {
	assert!(is_exec_teardown_noise(
		"read unixpacket @->/run/...: read: connection reset by peer"
	));
	// Ordinary program output is never suppressed.
	assert!(!is_exec_teardown_noise("connection reset by peer"));
	assert!(!is_exec_teardown_noise("hello world"));
}

#[test]
fn map_not_running_maps_404_and_stopped() {
	use crate::error::ComposeError;
	use crate::libpod::PodmanError;
	let e404 = PodmanError::Api {
		status: 404,
		message: "no such container: web".into(),
	};
	assert!(matches!(
		map_not_running(e404, "web"),
		ComposeError::NotRunning(s) if s == "web"
	));
	let e500 = PodmanError::Api {
		status: 500,
		message: "can only create exec sessions on running containers".into(),
	};
	assert!(matches!(
		map_not_running(e500, "web"),
		ComposeError::NotRunning(_)
	));
	// An unrelated error passes through unchanged.
	let other = PodmanError::Api {
		status: 500,
		message: "disk full".into(),
	};
	assert!(matches!(
		map_not_running(other, "web"),
		ComposeError::Podman(_)
	));
}

#[test]
fn exec_start_timeout_with_user_names_the_user() {
	use crate::libpod::PodmanError;
	// A client-side head timeout (the wedged-launch symptom) becomes a clear,
	// fast ExecFailed naming the likely culprit, never the raw socket-timeout.
	let timeout = PodmanError::Api {
		status: 0,
		message: format!(
			"timed out after {}s waiting for the Podman socket to respond",
			EXEC_START_TIMEOUT.as_secs()
		),
	};
	let opts = ExecOptions {
		user: Some("doesnotexist".into()),
		..Default::default()
	};
	let mapped = map_exec_start_err(timeout, &opts);
	match mapped {
		crate::error::ComposeError::ExecFailed(msg) => {
			assert!(msg.contains("doesnotexist"), "got {msg}");
			assert!(msg.contains("did not start"), "got {msg}");
			assert!(
				!msg.to_ascii_lowercase().contains("podman socket"),
				"must not leak the socket-timeout wording: {msg}"
			);
		}
		other => panic!("expected ExecFailed, got {other:?}"),
	}
}

#[test]
fn exec_start_timeout_without_user_names_the_workdir() {
	use crate::libpod::PodmanError;
	let timeout = PodmanError::Api {
		status: 0,
		message: "timed out after 20s waiting for the Podman socket to respond".into(),
	};
	let opts = ExecOptions {
		workdir: Some("/no/such/dir".into()),
		..Default::default()
	};
	match map_exec_start_err(timeout, &opts) {
		crate::error::ComposeError::ExecFailed(msg) => {
			assert!(msg.contains("/no/such/dir"), "got {msg}");
		}
		other => panic!("expected ExecFailed, got {other:?}"),
	}
}

#[test]
fn exec_start_real_api_error_passes_through() {
	use crate::error::ComposeError;
	use crate::libpod::PodmanError;
	// The prompt HTTP error an engine returns for a bad user is a genuine
	// diagnostic and must reach the user verbatim, not be rewritten.
	let api = PodmanError::Api {
		status: 500,
		message: "unable to find user doesnotexist: no matching entries in passwd file".into(),
	};
	let opts = ExecOptions {
		user: Some("doesnotexist".into()),
		..Default::default()
	};
	match map_exec_start_err(api, &opts) {
		ComposeError::Podman(e) => {
			assert!(e.to_string().contains("no matching entries in passwd file"));
		}
		other => panic!("expected Podman passthrough, got {other:?}"),
	}
}

/// The caller's standard input has to reach the command.
///
/// `exec -T db psql < dump.sql` never worked: the session was created with
/// no stdin attached, and the path that reads the response never wrote to
/// the session, so the file stayed on the host and the command saw an empty
/// stdin. The session is created with its stdin attached and the connection
/// is hijacked, so the bytes travel up the same connection the output comes
/// down on.
#[cfg(unix)]
#[tokio::test]
async fn exec_sends_the_callers_stdin_to_the_session() {
	use std::sync::{Arc, Mutex};

	use crate::compose::types::{ComposeFile, Service};
	use crate::engine::fake_podman::{self, FakeReply};
	use crate::engine::query::InputSource;
	use crate::engine::Engine;

	let received = Arc::new(Mutex::new(Vec::new()));
	let received_for_fake = received.clone();
	let fake = fake_podman::start_replying(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			FakeReply::Body(
				200,
				r#"[{"Names":["/proj-box-1"],"Labels":{"podup.service":"box"},"State":"running"}]"#
					.to_string(),
			)
		} else if method == "POST" && target.contains("/exec/exec-1/start") {
			// The hijacked session: the head, then the command's output.
			FakeReply::Hijacked {
				head: "HTTP/1.1 200 OK\r\n\
				       Content-Type: application/vnd.docker.multiplexed-stream\r\n\
				       \r\n"
					.to_string(),
				frames: vec![{
					let mut frame = vec![1u8, 0, 0, 0, 0, 0, 0, 2];
					frame.extend_from_slice(b"ok");
					frame
				}],
				received: received_for_fake.clone(),
			}
		} else if method == "POST" && target.contains("/containers/proj-box-1/exec") {
			FakeReply::Body(201, r#"{"Id":"exec-1"}"#.to_string())
		} else if method == "GET" && target.contains("/exec/exec-1/json") {
			FakeReply::Body(200, r#"{"ExitCode":0}"#.to_string())
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.to_string())
		}
	});

	let mut e = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	// The caller's input, as a redirected file would be.
	e.input = InputSource::Buffer(b"SELECT 1;\n".to_vec());

	let mut file = ComposeFile::default();
	file.services.insert("box".into(), Service::default());

	e.exec_with_options(
		&file,
		"box",
		vec!["psql".to_string()],
		ExecOptions::default().with_no_tty_for_test(true),
	)
	.await
	.expect("the exec session should end cleanly");

	assert_eq!(
		*received.lock().unwrap(),
		b"SELECT 1;\n",
		"the caller's input must reach the session"
	);

	// And the session has to be created with its stdin attached, or the
	// command never gets a reader for the bytes.
	let bodies = fake.bodies.lock().unwrap();
	assert!(
		bodies
			.iter()
			.any(|b| String::from_utf8_lossy(b).contains("\"AttachStdin\":true")),
		"the session must be created with its stdin attached: {bodies:?}"
	);
}
