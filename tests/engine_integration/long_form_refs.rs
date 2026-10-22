//! Engine integration tests for the long-form `source`/`target` form of a secret
//! or config reference, with the `mode` and `uid` it can carry.
use super::*;
// ---------------------------------------------------------------------------
// Secret/config long-form refs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn secret_long_form_ref() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("slf");
	let engine = Engine::new(client, proj.clone());
	// mode is octal notation per the Compose Specification (leading-zero `0400`);
	// uid is passed through to the native secret spec.
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    secrets:\n      - source: mysecret\n        target: /run/secrets/custom_name\n        mode: 0400\n        uid: \"0\"\nsecrets:\n  mysecret:\n    content: \"topsecret\"\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	// `cat` alone only proves the path exists: it exits 0 on an empty or wrong
	// file, and says nothing about the `mode:`/`uid:` the compose file asks for.
	// Check the content and the permissions the long form actually requested.
	let read = engine
		.exec_with_options(
			&file,
			"web",
			vec![
				"sh".to_string(),
				"-c".to_string(),
				"test \"$(cat /run/secrets/custom_name)\" = topsecret \
				 && test \"$(stat -c '%a %u' /run/secrets/custom_name)\" = '400 0'"
					.to_string(),
			],
			podup::ExecOptions::default(),
		)
		.await;
	engine.down(&file).await.unwrap();
	assert!(
		read.is_ok(),
		"the long-form secret did not land at the requested target with mode 0400 and uid 0: {read:?}"
	);
}

#[tokio::test]
async fn config_long_form_ref() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("clf");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    configs:\n      - source: mycfg\n        target: /etc/app.conf\nconfigs:\n  mycfg:\n    content: \"key=value\"\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	let read = engine
		.exec_with_options(
			&file,
			"web",
			vec![
				"sh".to_string(),
				"-c".to_string(),
				"test \"$(cat /etc/app.conf)\" = key=value".to_string(),
			],
			podup::ExecOptions::default(),
		)
		.await;
	engine.down(&file).await.unwrap();
	assert!(
		read.is_ok(),
		"the long-form config did not land at /etc/app.conf with its content: {read:?}"
	);
}
