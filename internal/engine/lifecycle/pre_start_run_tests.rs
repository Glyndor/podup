use super::pre_start_test_support::{created, engine, file, peer};
use super::{RunOptions, RunOverrides};

#[tokio::test]
async fn pre_start_run_uses_adjusted_service_before_the_run_container() {
	let peer = peer(0, false);
	let engine =
		engine(&peer).with_run_overrides(RunOverrides::default().with_user(Some("1234".into())));
	let options = RunOptions {
		detach: true,
		env_overrides: vec!["FROM_RUN=yes".into()],
		..RunOptions::default()
	};
	engine.run(&file(), "web", options).await.unwrap();
	let specs = created(&peer);
	assert_eq!(specs.len(), 2);
	assert!(specs[0]["name"].as_str().unwrap().contains("pre-start"));
	assert_eq!(specs[0]["user"], "1234");
	assert_eq!(specs[0]["env"]["FROM_RUN"], "yes");
	assert!(!specs[1]["name"].as_str().unwrap().contains("pre-start"));
	let seen = peer.requests.lock().unwrap();
	let wait = seen
		.iter()
		.position(|(path, _)| path.contains("/wait?"))
		.unwrap();
	let service_create = seen
		.iter()
		.rposition(|(path, _)| path.ends_with("/containers/create"))
		.unwrap();
	assert!(wait < service_create);
}

/// The command and the entrypoint a `run` was given describe the container
/// the run starts. An init step is a separate container with its own
/// command, so it keeps the service's.
#[tokio::test]
async fn pre_start_run_step_keeps_the_service_command_and_entrypoint() {
	let peer = peer(0, false);
	let engine = engine(&peer).with_run_overrides(RunOverrides {
		entrypoint: Some("sh".into()),
		..RunOverrides::default()
	});
	let file = crate::parse_str(
		"services:\n  web:\n    image: base\n    entrypoint: [/app]\n    pre_start:\n      - command: init\n",
	)
	.unwrap();
	let options = RunOptions {
		detach: true,
		cmd: vec!["echo".into()],
		..RunOptions::default()
	};
	engine.run(&file, "web", options).await.unwrap();
	let specs = created(&peer);
	assert_eq!(specs.len(), 2);
	assert_eq!(specs[0]["entrypoint"], serde_json::json!(["/app"]));
	assert_eq!(specs[0]["command"], serde_json::json!(["sh", "-c", "init"]));
	assert_eq!(specs[1]["entrypoint"], serde_json::json!(["sh"]));
	assert_eq!(specs[1]["command"], serde_json::json!(["echo"]));
}

/// An interactive `run` keeps STDIN open on the container it starts. The
/// init steps are not that container, so they do not get a terminal.
#[tokio::test]
async fn pre_start_run_step_does_not_inherit_the_interactive_flags() {
	let peer = peer(0, false);
	let engine = engine(&peer).with_run_overrides(RunOverrides {
		interactive: true,
		..RunOverrides::default()
	});
	let options = RunOptions {
		detach: true,
		..RunOptions::default()
	};
	engine.run(&file(), "web", options).await.unwrap();
	let specs = created(&peer);
	assert_eq!(specs.len(), 2);
	assert_eq!(specs[0]["stdin"], serde_json::Value::Null);
	assert_eq!(specs[0]["terminal"], serde_json::Value::Null);
	assert_eq!(specs[1]["stdin"], true);
}
