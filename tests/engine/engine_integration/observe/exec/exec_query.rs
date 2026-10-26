//! Engine integration tests for `exec`: the options it applies, and the errors it
//! reports for a service or a user that is not there.
use super::*;
#[tokio::test]
async fn exec_command_in_container() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("exc");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	let cname = format!("{proj}-web-1");
	// Leave the result on the container's filesystem. `echo` alone writes to a
	// stream the test cannot reach, so an exec that ran nowhere looked the same
	// as one that ran.
	engine
		.exec_with_options(
			&file,
			"web",
			vec![
				"sh".to_string(),
				"-c".to_string(),
				"echo exec-ran > /exec-marker".to_string(),
			],
			podup::ExecOptions::default(),
		)
		.await
		.unwrap();
	let marker = engine
		.test_exec_capture(&cname, vec!["cat".into(), "/exec-marker".into()])
		.await
		.unwrap_or_default();
	engine.down(&file).await.unwrap();

	assert_eq!(
		marker.trim(),
		"exec-ran",
		"exec returned success without running the command in the container"
	);
}

#[tokio::test]
async fn exec_with_options_user_workdir_env() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("excopt");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	let cname = format!("{proj}-web-1");
	// Write the three values out instead of printing them. A bad workdir or user
	// does make the exec error, so the old test caught those, but it could not
	// see an option that was accepted and then dropped on the floor: podup could
	// have sent no workdir at all and the exec would still have succeeded, in /.
	engine
		.exec_with_options(
			&file,
			"web",
			vec![
				"sh".to_string(),
				"-c".to_string(),
				"{ pwd; echo $FOO; id -un; } > /opts".to_string(),
			],
			podup::ExecOptions::new(
				vec!["FOO=bar".to_string()],
				Some("root".to_string()),
				Some("/tmp".to_string()),
				false,
				false,
				None,
				false,
			),
		)
		.await
		.unwrap();
	let opts = engine
		.test_exec_capture(&cname, vec!["cat".into(), "/opts".into()])
		.await
		.unwrap_or_default();
	engine.down(&file).await.unwrap();

	assert_eq!(
		opts.trim(),
		"/tmp\nbar\nroot",
		"exec did not apply workdir, env and user to the command"
	);
}

#[tokio::test]
async fn exec_unknown_service_fails() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("exf");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	let err = engine
		.exec_with_options(
			&file,
			"nonexistent",
			vec!["echo".to_string()],
			podup::ExecOptions::default(),
		)
		.await
		.unwrap_err();
	assert!(matches!(err, podup::ComposeError::ServiceNotFound(_)));
}

#[tokio::test]
async fn exec_nonexistent_user_fails_fast() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("exbu");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	// A nonexistent named user must surface a prompt, clear error, never hanging for
	// the full client read timeout (~120s) and then report a misleading
	// socket-timeout (issue #720).
	let started = std::time::Instant::now();
	let err = engine
		.exec_with_options(
			&file,
			"web",
			vec!["echo".to_string(), "hi".to_string()],
			podup::ExecOptions::default().with_user(Some("definitelynosuchuser".to_string())),
		)
		.await
		.unwrap_err();
	let elapsed = started.elapsed();
	engine.down(&file).await.unwrap();

	assert!(
		elapsed < std::time::Duration::from_secs(60),
		"exec with a bad user must fail fast, took {elapsed:?}"
	);
	let msg = err.to_string().to_ascii_lowercase();
	// Either the engine's prompt diagnostic (it names the user / passwd file) or
	// podup's exec-specific timeout message, but never the bare socket-timeout.
	assert!(
		msg.contains("user") || msg.contains("passwd") || msg.contains("exec"),
		"unexpected error for a bad exec user: {msg}"
	);
	assert!(
		!msg.contains("waiting for the podman socket"),
		"bad-user exec leaked a socket-timeout message: {msg}"
	);
	// A normal exec into the same service still works after the failure.
	engine.up(&file).await.unwrap();
	engine
		.exec_with_options(
			&file,
			"web",
			vec!["echo".to_string(), "ok".to_string()],
			podup::ExecOptions::default(),
		)
		.await
		.unwrap();
	engine.down(&file).await.unwrap();
}
