use super::pre_start_test_support::{created, engine, file, peer};
use super::{ExistingContainer, ResolvedImage};
use std::collections::HashMap;

#[tokio::test]
async fn pre_start_up_failure_preserves_the_previous_service() {
	let peer = peer(7, true);
	let engine = engine(&peer);
	let file = file();
	let existing = HashMap::from([(
		"proj-web".into(),
		ExistingContainer {
			config_hash: None,
			image_id: String::new(),
			service: Some("web".into()),
			oneoff: false,
		},
	)]);
	let error = engine
		.up_one_replica(
			"proj-web".into(),
			"web",
			&file.services["web"],
			&file,
			&existing,
			"new",
			&ResolvedImage::new(),
			false,
			true,
			true,
			&std::collections::BTreeMap::new(),
		)
		.await
		.unwrap_err();
	assert_eq!(
		error.to_string(),
		"service 'web': pre_start step 1: exited with code 7"
	);
	assert_eq!(created(&peer).len(), 1);
	let seen = peer.requests.lock().unwrap();
	assert!(!seen.iter().any(|(path, _)| path.contains("/proj-web/stop")
		|| path.contains("DELETE") && path.contains("/proj-web?")));
	assert!(!seen
		.iter()
		.any(|(path, _)| path.contains("/proj-web/start")));
	assert!(seen.iter().any(|(path, _)| path.contains("DELETE")
		&& path.contains("pre-start")
		&& path.contains("v=true")));
}

#[tokio::test]
async fn pre_start_up_initializes_stopped_reused_containers_only() {
	for no_recreate in [false, true] {
		for running in [false, true] {
			let peer = peer(0, running);
			let engine = engine(&peer);
			let file = file();
			let existing = HashMap::from([(
				"proj-web".into(),
				ExistingContainer {
					config_hash: Some("same".into()),
					image_id: "same-image".into(),
					service: Some("web".into()),
					oneoff: false,
				},
			)]);
			engine
				.up_one_replica(
					"proj-web".into(),
					"web",
					&file.services["web"],
					&file,
					&existing,
					"same",
					&ResolvedImage::new(),
					no_recreate,
					false,
					true,
					&std::collections::BTreeMap::new(),
				)
				.await
				.unwrap();
			assert_eq!(created(&peer).len(), usize::from(!running));
			let seen = peer.requests.lock().unwrap();
			let service_start = seen
				.iter()
				.position(|(path, _)| path.contains("/proj-web/start"))
				.unwrap();
			if !running {
				let wait = seen
					.iter()
					.position(|(path, _)| path.contains("/wait?"))
					.unwrap();
				assert!(wait < service_start);
			}
		}
	}
}

/// A `pre_start` step can write the `env_file` the service reads. The
/// environment resolved before it ran must not be what the container gets.
#[tokio::test]
async fn an_env_file_written_by_pre_start_reaches_the_container() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("generated.env"), "VERSION=old\n").unwrap();
	let file = crate::parse_str(
		"services:\n  web:\n    image: base\n    env_file: generated.env\n    pre_start:\n      - command: init\n",
	)
	.unwrap();
	let service = &file.services["web"];
	let peer = peer(0, false);
	let engine =
		crate::engine::Engine::with_base_dir(peer.client(), "proj".into(), dir.path().into());
	// What `up` resolves first, before the init step has run.
	let before = engine.effective_env(service).unwrap();
	assert_eq!(before["VERSION"], "old");
	// The init step rewrites the file, as the real one would.
	std::fs::write(dir.path().join("generated.env"), "VERSION=new\n").unwrap();
	engine
		.recreate_service("proj-web", "web", service, &file, true, &before)
		.await
		.unwrap();
	let specs = created(&peer);
	let web = specs.last().expect("the service container was created");
	let env = web["env"].to_string();
	assert!(env.contains("VERSION") && env.contains("new"), "{env}");
	assert!(
		!env.contains("old"),
		"the stale value reached the container: {env}"
	);
}
